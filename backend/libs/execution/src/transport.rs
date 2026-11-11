//! A reconnecting stream only replays observations, never commands.
use crate::Spool;
use ayni_orchestrator_telemetry::v1::execution_telemetry_service_client::ExecutionTelemetryServiceClient;
use std::{io, sync::Arc, time::Duration};
use tokio::sync::watch;
use tonic::{Request, transport::Endpoint};

/// # Errors
/// Reject invalid endpoint/metadata. Transport outages retry until shutdown.
pub async fn deliver(
    spool: Arc<Spool>,
    endpoint: &str,
    token: &str,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let endpoint = Endpoint::from_shared(endpoint.to_owned())
        .map_err(io::Error::other)?
        .connect_timeout(Duration::from_secs(5));
    let authorization: tonic::metadata::MetadataValue<tonic::metadata::Ascii> =
        format!("Bearer {token}")
            .parse()
            .map_err(io::Error::other)?;
    while !*shutdown.borrow() {
        let mut stop = shutdown.clone();
        let attempt = async {
            let channel = endpoint.connect().await.map_err(io::Error::other)?;
            let journal = spool.clone();
            let output = async_stream::stream! {
                let mut heartbeat = tokio::time::Instant::now();
                loop {
                    if heartbeat.elapsed() >= Duration::from_secs(10) && !crate::capture_finished() {
                        if let Some(capture) = crate::capture() {
                            let _ = capture.record("worker", "heartbeat", "", &[], None, None);
                        }
                        heartbeat = tokio::time::Instant::now();
                    }
                    if *stop.borrow() { break; }
                    match journal.pending() {
                        Ok(records) => for record in records { yield ayni_orchestrator_telemetry::v1::ExchangeTelemetryRequest { record: Some(record) }; },
                        Err(_) => break,
                    }
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_millis(250)) => {},
                        _ = stop.changed() => break,
                    }
                }
            };
            let mut request = Request::new(output);
            request
                .metadata_mut()
                .insert("authorization", authorization.clone());
            let mut incoming = ExecutionTelemetryServiceClient::new(channel)
                .exchange_telemetry(request)
                .await
                .map_err(io::Error::other)?
                .into_inner();
            while let Some(ack) = incoming.message().await.map_err(io::Error::other)? {
                spool.acknowledge(&ack.source_id, ack.sequence)?;
                if ack.cancel_requested {
                    crate::request_cancellation();
                }
            }
            Ok::<(), io::Error>(())
        };
        tokio::select! {
            _ = attempt => {},
            _ = shutdown.changed() => return Ok(()),
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(1)) => {},
            _ = shutdown.changed() => return Ok(()),
        }
    }
    Ok(())
}

pub struct Delivery {
    stop: watch::Sender<bool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    journals: Vec<Arc<Spool>>,
}
impl Delivery {
    /// Open trusted journals for this pod, including preceding isolated init containers.
    /// # Errors
    /// Return configuration and private journal failures.
    pub fn from_environment(token_path: &str) -> io::Result<Option<Self>> {
        let Some(capture) = crate::install_from_environment()? else {
            return Ok(None);
        };
        let Ok(endpoint) = std::env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") else {
            return Ok(None);
        };
        let token = std::fs::read_to_string(std::env::var(token_path).map_err(io::Error::other)?)?;
        let directory = std::env::var("ORCHESTRATOR_TELEMETRY_SPOOL").map_err(io::Error::other)?;
        let mut journals = vec![capture.spool];
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|name| name == capture.source.container.as_str())
            {
                continue;
            }
            let path = path.join("journal.sqlite");
            if path.is_file() {
                journals.push(Arc::new(Spool::open(&path, 128 * 1024 * 1024)?));
            }
        }
        let (stop, receiver) = watch::channel(false);
        let mut tasks = Vec::new();
        for journal in &journals {
            let journal = journal.clone();
            let endpoint = endpoint.clone();
            let token = token.clone();
            let receiver = receiver.clone();
            tasks.push(tokio::spawn(async move {
                let _ = deliver(journal, &endpoint, token.trim(), receiver).await;
            }));
        }
        Ok(Some(Self {
            stop,
            tasks,
            journals,
        }))
    }
    pub async fn finish(self) {
        if let Some(capture) = crate::capture() {
            let _ = capture.record("worker", "capture_finished", "", &[], None, None);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline
            && self
                .journals
                .iter()
                .any(|s| s.pending().is_ok_and(|p| !p.is_empty()))
        {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let _ = self.stop.send(true);
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// Replay existing observations only; this path cannot launch assigned actions.
/// # Errors
/// Retain the journal when delivery cannot finish in the bounded recovery attempt.
pub async fn replay(root: &std::path::Path, endpoint: &str, token: &str) -> io::Result<()> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            let nested = path.join("journal.sqlite");
            if nested.is_file() {
                paths.push(nested);
            }
        } else if path.file_name().is_some_and(|n| n == "journal.sqlite") {
            paths.push(path);
        }
    }
    for path in paths {
        let spool = Arc::new(Spool::open(&path, 128 * 1024 * 1024)?);
        let (stop, receiver) = watch::channel(false);
        let journal = spool.clone();
        let endpoint = endpoint.to_owned();
        let token = token.to_owned();
        let task = tokio::spawn(async move {
            let _ = deliver(journal, &endpoint, token.trim(), receiver).await;
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(540);
        while !spool.pending()?.is_empty() {
            if tokio::time::Instant::now() > deadline {
                let _ = stop.send(true);
                task.abort();
                return Err(io::Error::other("telemetry recovery pending"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let _ = stop.send(true);
        let _ = task.await;
    }
    Ok(())
}
