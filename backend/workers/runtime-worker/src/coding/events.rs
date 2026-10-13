//! Bounded supervisor snapshots. Advisory delivery never blocks coding or final delivery.
use super::{collect_usage, recovery};
use ayni_orchestrator_events::{
    EventIdentity, coding,
    v1::{CodingReportAvailable, CodingReportKind},
};
use ayni_orchestrator_runtime::{
    coding_events::{CodingAdmission, CodingSnapshot},
    runtime_execution::read_bounded,
};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct Reporter {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<io::Result<()>>,
}
impl Reporter {
    pub fn start(output: &Path, expected: Option<&CodingAdmission>) -> io::Result<Option<Self>> {
        let path = Path::new("/request/coding-reporting.json");
        if expected.is_none() {
            return Ok(None);
        }
        let mut admission: CodingAdmission =
            serde_json::from_slice(&read_bounded(path, 16_384)?).map_err(io::Error::other)?;
        admission
            .configuration
            .validate()
            .map_err(io::Error::other)?;
        let expected = expected.ok_or_else(|| io::Error::other("coding_admission_missing"))?;
        if expected.configuration.source != admission.configuration.source
            || expected.binding.generation != admission.binding.generation
            || expected.binding.snapshot_digest != admission.binding.snapshot_digest
            || admission.binding.request_digest
                != ayni_orchestrator_domain::digest::sha256(&read_bounded(
                    Path::new("/request/worker.json"),
                    65_536,
                )?)
        {
            return Err(io::Error::other("coding_admission_mismatch"));
        }
        admission.binding.pod_uid = std::env::var("POD_UID").map_err(io::Error::other)?;
        let output = output.to_owned();
        let (stop, mut receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let directory = output
                .join("coding-events")
                .join(&admission.binding.generation)
                .join(&admission.binding.pod_uid);
            std::fs::create_dir_all(&directory)?;
            let mut previous = Vec::new();
            let mut sequence = 0;
            loop {
                tokio::select! {
                    _ = &mut receive => break,
                    () = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
                if sequence >= 256 {
                    continue;
                }
                let Ok(snapshot) = snapshot(&output, &admission, false) else {
                    continue;
                };
                let bytes = serde_json::to_vec(&snapshot).map_err(io::Error::other)?;
                if bytes == previous {
                    continue;
                }
                sequence += 1;
                // Retain identity and bytes before attempting delivery. Timeout leaves a
                // durable snapshot; advisory loss cannot consume execution budget.
                let Ok(event) = retain(
                    &directory,
                    &admission,
                    sequence,
                    CodingReportKind::Progress,
                    &snapshot,
                ) else {
                    continue;
                };
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    deliver(&directory, &admission, event),
                )
                .await;
                previous = bytes;
            }
            let snapshot = snapshot(&output, &admission, true)?;
            let event = retain(
                &directory,
                &admission,
                257,
                CodingReportKind::Final,
                &snapshot,
            )?;
            tokio::time::timeout(
                Duration::from_secs(30),
                deliver(&directory, &admission, event),
            )
            .await
            .map_err(|_| io::Error::other("coding_reporting_unavailable"))?
        });
        Ok(Some(Self { stop, task }))
    }
    pub async fn finish(self) -> io::Result<()> {
        let _ = self.stop.send(());
        self.task.await.map_err(io::Error::other)?
    }
}
fn value(output: &Path, file: &str) -> io::Result<Option<serde_json::Value>> {
    match read_bounded(&output.join(file), 262_144) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).map_err(io::Error::other)?,
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn snapshot(
    output: &Path,
    admission: &CodingAdmission,
    terminal: bool,
) -> io::Result<CodingSnapshot> {
    Ok(CodingSnapshot {
        binding: admission.binding.clone(),
        progress: value(output, "progress.json")?
            .map(serde_json::from_value)
            .transpose()
            .map_err(io::Error::other)?,
        activity: value(output, "activity.json")?
            .map(serde_json::from_value)
            .transpose()
            .map_err(io::Error::other)?
            .unwrap_or_default(),
        usage: collect_usage(&output.join("usage"))?,
        publication_intent: value(output, "publication.json")?,
        publication_receipt: value(output, "publication.json")?
            .and_then(|j| j.get("receipt").cloned())
            .filter(|v| !v.is_null()),
        result: if terminal {
            Some(recovery::read_json(&output.join("result.json"))?)
        } else {
            None
        },
    })
}
fn retain(
    directory: &Path,
    admission: &CodingAdmission,
    sequence: u64,
    kind: CodingReportKind,
    snapshot: &CodingSnapshot,
) -> io::Result<(EventIdentity, Vec<u8>, PathBuf)> {
    let bytes = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
    if bytes.len() > coding::MAX_RECORD_BYTES {
        return Err(io::Error::other("coding_report_oversized"));
    }
    let payload = CodingReportAvailable {
        binding: Some(admission.binding.clone()),
        kind: kind as i32,
        sequence,
        sha256: ayni_orchestrator_domain::digest::sha256(&bytes),
        size_bytes: bytes.len() as u64,
        contract: coding::CONTRACT.into(),
    };
    let envelope = directory.join(format!("{sequence}.event.json"));
    if envelope.exists() {
        let encoded = read_bounded(&envelope, 16_384)?;
        let (identity, old) = coding::decode(&encoded).map_err(io::Error::other)?;
        if old != payload {
            return Err(io::Error::other("coding_result_conflict"));
        }
        return Ok((identity, encoded, envelope));
    }
    let identity = EventIdentity {
        source: admission.configuration.source.clone(),
        id: uuid::Uuid::now_v7().to_string(),
    };
    let encoded = coding::encode(&identity, &payload).map_err(io::Error::other)?;
    recovery::atomic_write(&directory.join(format!("{}.json", identity.id)), &bytes)?;
    crate::verification::reporting::persist(
        directory,
        &format!("{sequence}.event.json"),
        &encoded,
    )?;
    Ok((identity, encoded, envelope))
}
async fn deliver(
    directory: &Path,
    admission: &CodingAdmission,
    event: (EventIdentity, Vec<u8>, PathBuf),
) -> io::Result<()> {
    let (identity, bytes, _) = event;
    crate::verification::reporting::deliver_event(
        directory,
        &admission.configuration,
        Path::new("/reporting/token"),
        &identity,
        bytes,
        &format!("{}.receipt.json", identity.id),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::v1::CodingExecutionBinding;
    #[test]
    fn retained_terminal_identity_replays_and_conflicting_content_cannot_replace_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let admission = CodingAdmission {
            configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                source: "ayni://orchestrator/supervisors/test".into(),
                endpoint: "http://127.0.0.1/events/verification".into(),
            },
            binding: CodingExecutionBinding {
                task_id: "task-one".into(),
                attempt_id: "attempt-one".into(),
                generation: "generation-one".into(),
                execution_id: "coding-one".into(),
                pod_uid: "pod-one".into(),
                snapshot_digest: format!("sha256:{}", "a".repeat(64)),
                request_digest: format!("sha256:{}", "b".repeat(64)),
            },
        };
        let mut snapshot = CodingSnapshot {
            binding: admission.binding.clone(),
            progress: None,
            activity: vec![],
            usage: serde_json::Value::Null,
            publication_intent: None,
            publication_receipt: None,
            result: Some(super::super::empty_result()),
        };
        let first = retain(
            directory.path(),
            &admission,
            257,
            CodingReportKind::Final,
            &snapshot,
        )?;
        let repeated = retain(
            directory.path(),
            &admission,
            257,
            CodingReportKind::Final,
            &snapshot,
        )?;
        assert_eq!(first, repeated);
        let (_, payload) = coding::decode(&first.1)?;
        let retained = std::fs::read(directory.path().join(format!("{}.json", first.0.id)))?;
        assert_eq!(
            payload.sha256,
            ayni_orchestrator_domain::digest::sha256(&retained)
        );
        snapshot
            .result
            .as_mut()
            .ok_or("result")?
            .coding_turns_consumed = 1;
        assert!(
            retain(
                directory.path(),
                &admission,
                257,
                CodingReportKind::Final,
                &snapshot
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(directory.path().join(format!("{}.json", first.0.id)))?,
            retained
        );
        Ok(())
    }
}
