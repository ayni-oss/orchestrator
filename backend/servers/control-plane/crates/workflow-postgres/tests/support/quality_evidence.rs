use super::{QualityOutcome, TaskOutcome, snapshot_digest};

pub fn failed_quality(
    command: &ayni_orchestrator_events::task_execution::TaskCommand,
    outcome: &TaskOutcome,
) -> QualityOutcome {
    let hash = format!("sha256:{}", "e".repeat(64));
    let mut artifact = serde_json::json!({
        "schema_version":"0.5.0", "invocation":{"command":"check","scope":{}}, "execution_mode":"managed",
        "environment_lock_fingerprint":hash,"source_fingerprint":hash,"contract_digest":hash,
        "prebuilt_runtime":{"metadata_digest":hash,"certificate":{"certificate":{"platform":"linux/arm64","lock_fingerprint":hash}}},
        "tool_versions":[{"tool":"rust","version":"1.94.0"}],
        "completion":{"scope":"repository","state":"complete","expected_targets":1,"completed_targets":1,"detected_targets":1,"skipped_targets":0,"issues":[]},
        "aggregate":{"status":"fail","total_rows":1,"passing_rows":0,"failing_rows":1},"rows":[{"pass":false}]
    }).to_string();
    artifact.push_str(&" ".repeat(256 * 1024));
    assert!(
        artifact.len() > 262_144,
        "exercise complete repository-sized evidence"
    );
    QualityOutcome {
        state: "failed".into(),
        safe_reason: "quality_failed".into(),
        snapshot_digest: snapshot_digest(command, outcome),
        lock_digest: command.lock_digest.clone(),
        runtime_image: command.runtime_image.clone(),
        runtime_platform: "linux/arm64".into(),
        runtime_metadata_digest: hash.clone(),
        executor_digest: hash.clone(),
        contract_digest: hash.clone(),
        lock_fingerprint: hash.clone(),
        source_fingerprint: hash.clone(),
        validation_input_digest: hash,
        exit_code: Some(1),
        artifact_digest: ayni_orchestrator_events::task_execution::digest(artifact.as_bytes()),
        artifact,
        expected_targets: 1,
        completed_targets: 1,
    }
}

/// Both raw artifact and complete encoded-envelope limits reject before evidence retention.
pub async fn reject_excess(
    store: &super::WorkflowStore,
    task: &ayni_orchestrator_workflow_postgres::execution::TaskExecutionPreparation,
    uid: &str,
    quality: &QualityOutcome,
) -> super::Result {
    use ayni_orchestrator_events::{
        task_execution::{WorkerRequest, digest},
        task_quality::{MAX_ARTIFACT, MAX_QUALITY_OUTCOME_BYTES},
    };
    let mut artifact = quality.clone();
    artifact.artifact.push_str(&" ".repeat(MAX_ARTIFACT));
    artifact.artifact_digest = digest(artifact.artifact.as_bytes());
    let mut envelope = quality.clone();
    envelope.state = "blocked".into();
    envelope.safe_reason = "runtime_unavailable".into();
    envelope.artifact.clear();
    envelope.artifact_digest.clear();
    envelope.expected_targets = 0;
    envelope.completed_targets = 0;
    envelope.runtime_platform = "x".repeat(MAX_QUALITY_OUTCOME_BYTES);
    for outcome in [artifact, envelope] {
        assert!(matches!(
            store
                .task_worker(
                    &task.attempt_id,
                    1,
                    uid,
                    &WorkerRequest::QualityReport { outcome }
                )
                .await,
            Err(super::StoreError::InvalidInput("task_quality"))
        ));
    }
    Ok(())
}
