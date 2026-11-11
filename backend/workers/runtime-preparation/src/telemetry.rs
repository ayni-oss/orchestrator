//! Runtime telemetry survives the stage process in a separate attempt volume.
use ayni_orchestrator_events::runtime_preparation::Binding;
use ayni_orchestrator_execution::{Capture, Spool, now_ms};
use ayni_orchestrator_telemetry::v1::LogSource;
use std::{env, io, path::Path, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

pub struct Delivery {
    spool: Arc<Spool>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Delivery {
    /// # Errors
    /// A configured journal must be available; absence of configuration disables capture.
    pub fn start(binding: &Binding, token: String) -> io::Result<Option<Self>> {
        let Ok(endpoint) = env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") else {
            return Ok(None);
        };
        let directory = env::var("ORCHESTRATOR_TELEMETRY_SPOOL").map_err(io::Error::other)?;
        let spool = Arc::new(Spool::open(
            &Path::new(&directory).join("journal.sqlite"),
            env::var("ORCHESTRATOR_TELEMETRY_SPOOL_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(128 * 1024 * 1024),
        )?);
        let source = LogSource {
            id: format!(
                "worker:{}:{}:{}",
                binding.pod_uid, binding.stage, binding.generation
            ),
            deployment: env::var("ORCHESTRATOR_DEPLOYMENT").unwrap_or_default(),
            namespace: env::var("ORCHESTRATOR_POD_NAMESPACE").unwrap_or_default(),
            pod_name: env::var("ORCHESTRATOR_POD_NAME").unwrap_or_default(),
            pod_uid: binding.pod_uid.clone(),
            container: "stage".into(),
            component: "runtime-worker".into(),
            execution_id: binding.operation_id.clone(),
            generation: binding.generation,
            stage: binding.stage.clone(),
            provenance: "worker".into(),
            capture_status: "pending_delivery".into(),
            observed_at_unix_ms: now_ms(),
            ..Default::default()
        };
        ayni_orchestrator_execution::install(Capture {
            spool: spool.clone(),
            source,
            command_digest: binding.command_digest.clone(),
            raw: env::var("ORCHESTRATOR_DEBUG").is_ok_and(|v| v == "true"),
        });
        let (stop, receiver) = watch::channel(false);
        let journal = spool.clone();
        let task = tokio::spawn(async move {
            let _ = ayni_orchestrator_execution::transport::deliver(
                journal, &endpoint, &token, receiver,
            )
            .await;
        });
        Ok(Some(Self { spool, stop, task }))
    }
    pub async fn finish(self) {
        if let Some(capture) = ayni_orchestrator_execution::capture() {
            let _ = capture.record("worker", "capture_finished", "", &[], None, None);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            if self.spool.pending().is_ok_and(|records| records.is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

/// Replay only retained observations from a trusted recovery pod.
/// # Errors
/// Return journal/configuration errors; leave the PVC for another recovery attempt.
pub async fn replay() -> io::Result<()> {
    let root = env::var("ORCHESTRATOR_TELEMETRY_SPOOL").map_err(io::Error::other)?;
    let endpoint = env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT").map_err(io::Error::other)?;
    let token = std::fs::read_to_string(
        env::var("ORCHESTRATOR_RUNTIME_CAPABILITY_PATH").map_err(io::Error::other)?,
    )?;
    ayni_orchestrator_execution::transport::replay(
        std::path::Path::new(&root),
        &endpoint,
        token.trim(),
    )
    .await
}
