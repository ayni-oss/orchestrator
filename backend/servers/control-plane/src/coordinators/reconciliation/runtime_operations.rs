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
        operations::MAX_RECORD_BYTES,
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

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn discover_operation(
        &self,
        id: &str,
        cursor: &mut RuntimeCursor,
        runtime: WorkloadRuntime,
        name: &str,
    ) -> Result<super::ImageDiscovery, &'static str> {
        let workload = self
            .operation_runtime(id, cursor, runtime, name, RuntimeOperationKind::Discovery)
            .await?;
        self.kubernetes.start_discovery(name, &workload).await?;
        if workload.operation_reporting.is_none() {
            return Ok(self.kubernetes.discover(name).await?);
        }
        match self
            .operation_result(cursor, name, RuntimeOperationKind::Discovery)
            .await?
        {
            Some(OperationOutcome::Discovery { result }) => {
                if matches!(result, super::ImageDiscovery::Failed { .. }) {
                    self.save(id, cursor).await?;
                }
                Ok(result)
            }
            Some(OperationOutcome::Failed { reason }) => {
                self.save(id, cursor).await?;
                Ok(super::ImageDiscovery::Failed { reason })
            }
            Some(_) => Err("runtime_result_invalid"),
            None => Ok(super::ImageDiscovery::Pending),
        }
    }
    pub(super) async fn runtime_operation_state(
        &self,
        cursor: &mut RuntimeCursor,
        name: &str,
        kind: RuntimeOperationKind,
    ) -> Result<
        (
            ayni_orchestrator_application::execution::ExecutionState,
            Option<OperationOutcome>,
        ),
        &'static str,
    > {
        use ayni_orchestrator_application::execution::ExecutionState as S;
        let result = self.operation_result(cursor, name, kind).await?;
        let state = match &result {
            Some(OperationOutcome::Failed { .. }) => S::Failed,
            Some(_) => S::Succeeded,
            None => self.kubernetes.coding_state(name).await?,
        };
        if result.is_none()
            && matches!(state, S::Succeeded | S::Failed)
            && kind != RuntimeOperationKind::Publication
        {
            return Err("runtime_report_missing");
        }
        Ok((state, result))
    }
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn operation_build_state(
        &self,
        cursor: &mut RuntimeCursor,
        pending: &crate::profiles::ayni::preparation_state::PendingVerification,
        name: &str,
        workload: &WorkloadRuntime,
    ) -> Result<ayni_orchestrator_application::image_build::BuildObservation, &'static str> {
        use ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase;
        use ayni_orchestrator_application::{
            execution::ExecutionState, image_build::BuildObservation,
        };
        if workload.operation_reporting.is_none() {
            return Ok(ayni_orchestrator_application::image_build::observe_build(
                &self.kubernetes,
                name,
                workload,
                pending.phase.build_started(),
            )
            .await?);
        }
        if pending.phase.build_started() && !self.kubernetes.engine_alive(name, workload).await? {
            return Ok(BuildObservation::EngineLost);
        }
        if pending.phase == ImagePreparationPhase::Publishing {
            return Ok(BuildObservation::Running);
        }
        match self
            .operation_result(cursor, name, RuntimeOperationKind::Build)
            .await?
        {
            Some(OperationOutcome::Built { candidate_image_id }) => {
                Ok(BuildObservation::Completed(candidate_image_id))
            }
            Some(OperationOutcome::Failed { reason }) => Ok(BuildObservation::Failed(reason)),
            Some(_) => Err("runtime_result_invalid"),
            None => match self.kubernetes.coding_state(name).await? {
                ExecutionState::Missing => Ok(BuildObservation::Missing),
                ExecutionState::Running => Ok(BuildObservation::Running),
                _ => Err("runtime_report_missing"),
            },
        }
    }
}
