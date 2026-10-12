//! Source-aware runtime result collection; receipt and progression remain separate.
use super::{ExecutionBackend, RuntimeCursor, RuntimeReconciler};
use ayni_orchestrator_application::execution::WorkloadRuntime;
use ayni_orchestrator_events::{
    operations,
    v1::{RuntimeOperationBinding, RuntimeOperationKind},
};
use ayni_orchestrator_runtime::operation::{
    OperationAdmission, OperationOutcome, OperationRecord, RECORD_FILE,
};

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn operation_runtime(
        &self,
        id: &str,
        cursor: &RuntimeCursor,
        mut runtime: WorkloadRuntime,
        execution: &str,
        kind: RuntimeOperationKind,
    ) -> Result<WorkloadRuntime, &'static str> {
        let Some(preparation) = &cursor.preparation else {
            return Ok(runtime);
        };
        let Some(configuration) = &preparation.operation_reporting else {
            return Ok(runtime);
        };
        let profile = id.to_owned();
        let profile = self
            .with_database(move |store| {
                store
                    .runtime_profile(&profile)
                    .map_err(|_| "runtime_profile_missing")
            })
            .await?;
        runtime.operation_reporting = Some(OperationAdmission {
            configuration: configuration.clone(),
            kind,
            binding: RuntimeOperationBinding {
                repository_connection_id: profile.repository_connection_id,
                runtime_profile_id: id.into(),
                runtime_profile_revision_id: profile
                    .active_revision_id
                    .ok_or("runtime_profile_missing")?,
                preparation_id: preparation.generation.clone(),
                execution_id: execution.into(),
                pod_uid: String::new(),
                request_digest: String::new(),
                source_revision: preparation.source_revision.clone(),
                lock_digest: preparation.lock_digest.clone(),
                policy_digest: preparation.policy_digest.clone(),
                platform: preparation.platform.clone(),
            },
        });
        Ok(runtime)
    }

    pub(super) async fn operation_result(
        &self,
        cursor: &mut RuntimeCursor,
        execution: &str,
        kind: RuntimeOperationKind,
    ) -> Result<Option<OperationOutcome>, &'static str> {
        let execution = execution.to_owned();
        let receipt = self
            .with_database(move |store| {
                store
                    .runtime_operation_receipt(&execution)
                    .map_err(|_| "runtime_operation_receipt_invalid")
            })
            .await?;
        let Some((identity, payload)) = receipt else {
            return Ok(None);
        };
        let b = payload.binding.as_ref().ok_or("runtime_result_invalid")?;
        let preparation = cursor
            .preparation
            .as_ref()
            .ok_or("runtime_preparation_missing")?;
        if payload.kind != kind as i32
            || b.preparation_id != preparation.generation
            || b.source_revision != preparation.source_revision
            || b.policy_digest != preparation.policy_digest
            || b.lock_digest != preparation.lock_digest
            || b.platform != preparation.platform
            || preparation
                .operation_reporting
                .as_ref()
                .is_none_or(|r| r.source != identity.source)
        {
            return Err("runtime_result_mismatch");
        }
        let execution_id = b.execution_id.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            read_record(std::path::Path::new("/runtime-outputs"), &payload)
        })
        .await
        .map_err(|_| "runtime_report_unavailable")??;
        cursor
            .preparation
            .as_mut()
            .ok_or("runtime_preparation_missing")?
            .operation_observations
            .insert(execution_id, identity);
        Ok(Some(outcome))
    }
}

fn read_record(
    root: &std::path::Path,
    payload: &ayni_orchestrator_events::v1::RuntimeOperationResultAvailable,
) -> Result<OperationOutcome, &'static str> {
    let b = payload.binding.as_ref().ok_or("runtime_result_invalid")?;
    let e = payload.evidence.as_ref().ok_or("runtime_result_invalid")?;
    let directory = root.join(&b.execution_id);
    if !std::fs::symlink_metadata(&directory)
        .map_err(|_| "runtime_report_unavailable")?
        .is_dir()
    {
        return Err("runtime_result_invalid");
    }
    let bytes = ayni_orchestrator_runtime::runtime_execution::read_bounded(
        &directory.join(RECORD_FILE),
        operations::MAX_RECORD_BYTES as usize,
    )
    .map_err(|_| "runtime_report_unavailable")?;
    if bytes.len() as u64 != e.size_bytes
        || ayni_orchestrator_domain::digest::sha256(&bytes) != e.sha256
    {
        return Err("runtime_result_invalid");
    }
    let record: OperationRecord =
        serde_json::from_slice(&bytes).map_err(|_| "runtime_result_invalid")?;
    let kind =
        RuntimeOperationKind::try_from(payload.kind).map_err(|_| "runtime_result_invalid")?;
    if record.contract != operations::CONTRACT
        || record.binding != *b
        || !record.outcome.matches(kind)
    {
        return Err("runtime_result_invalid");
    }
    Ok(record.outcome)
}
