//! Consume receipts independently of Pod completion; stop confirmation still fences writers.
use super::{
    ExecutionBackend, RuntimeReconciler,
    coding::{coding_usage_reports, milliseconds, validate_worker_evidence},
    v1, worker_mapping,
};
use ayni_orchestrator_events::{
    EventIdentity, coding,
    v1::{CodingReportAvailable, CodingReportKind},
};
use ayni_orchestrator_runtime::{coding_events::CodingSnapshot, runtime_execution::read_bounded};
use std::path::Path;
impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn recover_coding_receipt(&self, name: &str) -> Result<(), &'static str> {
        let Some(admission) = self.kubernetes.coding_reporting_identity(name).await? else {
            return Ok(());
        };
        let path = Path::new("/runtime-outputs")
            .join(name)
            .join("coding-events")
            .join(&admission.binding.generation)
            .join(&admission.binding.pod_uid)
            .join("257.event.json");
        let event = tokio::task::spawn_blocking(move || {
            read_bounded(&path, ayni_orchestrator_events::MAX_EVENT_BYTES)
        })
        .await
        .map_err(|_| "coding_report_unavailable")?;
        let Ok(bytes) = event else {
            return Ok(());
        };
        let (identity, payload) = coding::decode(&bytes).map_err(|_| "coding_report_invalid")?;
        if payload.binding.as_ref() != Some(&admission.binding)
            || identity.source != admission.configuration.source
        {
            return Err("coding_report_invalid");
        }
        self.with_database(move |s| {
            s.receive_coding(&identity, &payload)
                .map_err(|_| "coding_receipt_invalid")
        })
        .await
    }
    pub(super) async fn consume_coding_reports(
        &self,
        attempt: &v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<(bool, bool), &'static str> {
        let id = attempt.id.clone();
        let pending = self
            .with_database(move |s| s.pending_coding(&id).map_err(|_| "coding_receipt_invalid"))
            .await?;
        let mut terminal = false;
        let mut interrupted = false;
        for (identity, payload) in pending {
            let event = identity.clone();
            let report = payload.clone();
            let name = name.to_owned();
            let snapshot = tokio::task::spawn_blocking(move || {
                read_snapshot(Path::new("/runtime-outputs"), &name, &event, &report)
            })
            .await
            .map_err(|_| "coding_report_unavailable")?;
            let snapshot = match snapshot {
                Ok(s) => s,
                Err("coding_report_unavailable") => continue,
                Err(_) => {
                    self.with_database(move |s| {
                        s.reject_coding_report(&identity)
                            .map_err(|_| "coding_receipt_invalid")
                    })
                    .await?;
                    continue;
                }
            };
            if snapshot.result.as_ref().is_some_and(|r| {
                r.execution == ayni_orchestrator_runtime::coding::ExecutionOutcome::Cancelled
            }) {
                // SIGTERM without durable cancellation is recovery, not task cancellation.
                let id = attempt.id.clone();
                let binding = payload.binding.clone().ok_or("coding_receipt_invalid")?;
                let current = self
                    .with_database(move |s| {
                        s.coding_admission(&id)
                            .map_err(|_| "coding_authority_invalid")
                    })
                    .await?;
                if current
                    .as_ref()
                    .is_some_and(|a| a.binding.generation == binding.generation)
                    && [
                        v1::DevelopmentAttemptState::Running as i32,
                        v1::DevelopmentAttemptState::Queued as i32,
                    ]
                    .contains(&attempt.state)
                {
                    interrupted = true;
                    continue;
                }
            }
            let reports = serde_json::from_value(if snapshot.usage.is_null() {
                serde_json::json!([])
            } else {
                snapshot.usage
            });
            let Ok(usage) = reports
                .map_err(|_| "coding_usage_invalid")
                .and_then(coding_usage_reports)
            else {
                self.with_database(move |store| {
                    store
                        .reject_coding_report(&identity)
                        .map_err(|_| "coding_receipt_invalid")
                })
                .await?;
                continue;
            };
            let progress = snapshot.progress.map(worker_mapping::progress);
            let result = snapshot
                .result
                .map(|r| worker_mapping::result(r, usage.clone()));
            let activity = snapshot
                .activity
                .into_iter()
                .map(|a| v1::AgentActivity {
                    sequence: a.sequence,
                    observed_at: Some(milliseconds(a.timestamp_ms)),
                    message: a.failure.map_or_else(
                        || a.stage.message().to_owned(),
                        |f| format!("{}: {}", a.stage.message(), f.message()),
                    ),
                    failed: a.failed,
                })
                .collect();
            let final_report = payload.kind == CodingReportKind::Final as i32;
            let applied = self
                .with_database(move |s| {
                    s.commit_coding_report(&identity, &payload, progress, result, activity, usage)
                        .map_err(|_| "coding_report_rejected")
                })
                .await?;
            terminal |= applied && final_report;
            if terminal {
                return Ok((true, interrupted));
            }
        }
        Ok((terminal, interrupted))
    }
}
fn read_snapshot(
    root: &Path,
    name: &str,
    identity: &EventIdentity,
    payload: &CodingReportAvailable,
) -> Result<CodingSnapshot, &'static str> {
    coding::encode(identity, payload).map_err(|_| "coding_report_invalid")?;
    let b = payload.binding.as_ref().ok_or("coding_report_invalid")?;
    if b.execution_id != name {
        return Err("coding_report_invalid");
    }
    let directory = root.join(name);
    let retained = directory
        .join("coding-events")
        .join(&b.generation)
        .join(&b.pod_uid);
    for p in [
        &directory,
        directory.join("coding-events").as_path(),
        directory
            .join("coding-events")
            .join(&b.generation)
            .as_path(),
        &retained,
    ] {
        if !std::fs::symlink_metadata(p)
            .map_err(|_| "coding_report_unavailable")?
            .is_dir()
        {
            return Err("coding_report_invalid");
        }
    }
    let bytes = read_bounded(
        &retained.join(format!("{}.json", identity.id)),
        coding::MAX_RECORD_BYTES,
    )
    .map_err(|_| "coding_report_unavailable")?;
    if bytes.len() as u64 != payload.size_bytes
        || ayni_orchestrator_domain::digest::sha256(&bytes) != payload.sha256
    {
        return Err("coding_report_invalid");
    }
    let snapshot: CodingSnapshot =
        serde_json::from_slice(&bytes).map_err(|_| "coding_report_invalid")?;
    if snapshot.binding != *b
        || (snapshot.result.is_some() != (payload.kind == CodingReportKind::Final as i32))
        || !ayni_orchestrator_runtime::activity::valid(&snapshot.activity)
    {
        return Err("coding_report_invalid");
    }
    if let Some(r) = &snapshot.result {
        validate_worker_evidence(&directory, r)?;
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_snapshot_rejects_tampering_wrong_execution_and_directory_symlinks()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let (identity, mut report) = coding::decode(include_bytes!(
            "../../../../../../proto/testdata/coding/final.json"
        ))?;
        report.kind = CodingReportKind::Progress as i32;
        report.sequence = 1;
        let binding = report.binding.clone().ok_or("binding")?;
        let snapshot = CodingSnapshot {
            binding: binding.clone(),
            progress: None,
            activity: Vec::new(),
            usage: serde_json::Value::Null,
            publication_intent: None,
            publication_receipt: None,
            result: None,
        };
        let retained = root
            .path()
            .join(&binding.execution_id)
            .join("coding-events")
            .join(&binding.generation)
            .join(&binding.pod_uid);
        std::fs::create_dir_all(&retained)?;
        let path = retained.join(format!("{}.json", identity.id));
        let bytes = serde_json::to_vec(&snapshot)?;
        report.sha256 = ayni_orchestrator_domain::digest::sha256(&bytes);
        report.size_bytes = bytes.len() as u64;
        std::fs::write(&path, &bytes)?;
        assert!(read_snapshot(root.path(), &binding.execution_id, &identity, &report).is_ok());
        assert!(read_snapshot(root.path(), "other-job", &identity, &report).is_err());
        std::fs::write(&path, b"{}")?;
        assert!(read_snapshot(root.path(), &binding.execution_id, &identity, &report).is_err());
        std::fs::write(&path, &bytes)?;
        #[cfg(unix)]
        {
            let moved = root.path().join("moved");
            std::fs::rename(&retained, &moved)?;
            std::os::unix::fs::symlink(&moved, &retained)?;
            assert!(read_snapshot(root.path(), &binding.execution_id, &identity, &report).is_err());
        }
        Ok(())
    }
}
