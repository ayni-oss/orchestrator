//! Shared command ownership and durable diagnostic transport.
pub mod files;
pub mod process;
pub mod spool;
pub mod transport;
use ayni_orchestrator_telemetry::v1::{LogSource, TelemetryRecord};
pub use spool::Spool;
use std::{
    io,
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

tokio::task_local! { static ACTION_CAPTURE: Capture; }

static CAPTURE: OnceLock<Capture> = OnceLock::new();
static FINISHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[must_use]
pub fn capture_finished() -> bool {
    FINISHED.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone)]
pub struct Capture {
    pub spool: Arc<Spool>,
    pub source: LogSource,
    pub command_digest: String,
    pub raw: bool,
}

#[must_use]
pub fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

impl Capture {
    /// # Errors
    /// Returns a spool failure; callers must continue draining child pipes.
    pub fn record(
        &self,
        action: &str,
        event: &str,
        stream: &str,
        data: &[u8],
        exit_code: Option<i32>,
        signal: Option<i32>,
    ) -> io::Result<()> {
        if !self.raw && !data.is_empty() {
            return Ok(());
        }
        let mut source = self.source.clone();
        source.observed_at_unix_ms = now_ms();
        if event == "drain_incomplete" {
            let _ = self
                .spool
                .set_checkpoint(&format!("loss:{}", source.id), "true");
        }
        if event == "capture_finished" {
            source.capture_status = if self
                .spool
                .checkpoint(&format!("loss:{}", source.id))?
                .is_some()
            {
                "delivery_finished_with_gaps"
            } else {
                "delivery_finished"
            }
            .into();
            FINISHED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.spool.append(TelemetryRecord {
            source: Some(source),
            pending_bytes: if event == "capture_finished" {
                0
            } else {
                self.spool.pending_bytes().unwrap_or_default()
            },
            parent_action_id: {
                let root = format!(
                    "{}:{}:{}",
                    self.source.execution_id, self.source.stage, self.source.generation
                );
                if root == action { String::new() } else { root }
            },
            action_id: action.into(),
            event: event.into(),
            stream: stream.into(),
            data: data.into(),
            observed_at_unix_ms: now_ms(),
            exit_code,
            signal,
            command_digest: self.command_digest.clone(),
            ..Default::default()
        })
    }
}

/// Install once in a trusted worker parent, never in a repository child.
pub fn install(capture: Capture) {
    if CAPTURE.set(capture).is_ok()
        && let Some(capture) = CAPTURE.get()
    {
        let root = format!(
            "{}:{}:{}",
            capture.source.execution_id, capture.source.stage, capture.source.generation
        );
        let _ = capture.record(&root, "started", "", &[], None, None);
    }
}
#[must_use]
pub fn capture() -> Option<Capture> {
    ACTION_CAPTURE
        .try_with(Clone::clone)
        .ok()
        .or_else(|| CAPTURE.get().cloned())
}

/// Correlate a trusted in-process action and its child commands with assigned work.
pub async fn scope<F: std::future::Future>(capture: Capture, future: F) -> F::Output {
    ACTION_CAPTURE.scope(capture, future).await
}

/// Configure a trusted local capture without establishing network access.
/// # Errors
/// Return malformed source metadata or inaccessible private storage.
pub fn install_from_environment() -> io::Result<Option<Capture>> {
    let Ok(metadata) = std::env::var("ORCHESTRATOR_CAPTURE_SOURCE") else {
        return Ok(None);
    };
    let mut source: LogSource = serde_json::from_str(&metadata).map_err(io::Error::other)?;
    source.pod_uid = std::env::var("ORCHESTRATOR_CAPTURE_POD_UID").map_err(io::Error::other)?;
    source.pod_name = std::env::var("ORCHESTRATOR_CAPTURE_POD_NAME").unwrap_or_default();
    source.namespace = std::env::var("ORCHESTRATOR_CAPTURE_NAMESPACE").unwrap_or_default();
    source.id = format!(
        "worker:{}:{}:{}",
        source.pod_uid, source.container, source.generation
    );
    source.observed_at_unix_ms = now_ms();
    source.provenance = "worker".into();
    source.capture_status = "pending_delivery".into();
    let directory = std::env::var("ORCHESTRATOR_TELEMETRY_SPOOL").map_err(io::Error::other)?;
    let directory = std::path::Path::new(&directory).join(&source.container);
    std::fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            &directory,
            std::fs::Permissions::from_mode(if source.component == "runtime-worker" {
                0o700
            } else {
                0o770
            }),
        )?;
    }
    let maximum = std::env::var("ORCHESTRATOR_TELEMETRY_SPOOL_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128 * 1024 * 1024);
    let capture = Capture {
        spool: Arc::new(Spool::open(&directory.join("journal.sqlite"), maximum)?),
        source,
        command_digest: std::env::var("ORCHESTRATOR_CAPTURE_COMMAND_DIGEST").unwrap_or_default(),
        raw: std::env::var("ORCHESTRATOR_DEBUG").is_ok_and(|v| v == "true"),
    };
    install(capture.clone());
    Ok(Some(capture))
}

static CANCEL: OnceLock<tokio::sync::watch::Sender<bool>> = OnceLock::new();
fn cancellation() -> &'static tokio::sync::watch::Sender<bool> {
    CANCEL.get_or_init(|| tokio::sync::watch::channel(false).0)
}
pub fn request_cancellation() {
    cancellation().send_replace(true);
}
#[must_use]
pub fn is_cancelled() -> bool {
    *cancellation().borrow()
}
pub async fn cancelled() {
    let mut signal = cancellation().subscribe();
    if !*signal.borrow() {
        let _ = signal.changed().await;
    }
}

#[cfg(test)]
mod transport_tests;
