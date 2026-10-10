//! Conversion from the public wire request into application-owned task inputs.
use super::{Message, v1};
use crate::adapters::sqlite::{CleanStoreError, task_admission::admission_error};
use ayni_orchestrator_application::tasks::{
    AdmitTask, AgentSelection, DeadlineDuration, RequestIdentity,
};
use ayni_orchestrator_domain::ChangeConstraints;

pub fn task_command(
    key: &str,
    request: &v1::CreateDevelopmentTaskRequest,
) -> Result<AdmitTask, CleanStoreError> {
    let requested = request
        .request
        .as_ref()
        .ok_or(CleanStoreError::CorruptState)?;
    let payload = request.encode_to_vec();
    Ok(AdmitTask {
        identity: RequestIdentity {
            key: key.into(),
            digest: crate::adapters::sqlite::request_digest("CreateDevelopmentTask", request),
            payload,
        },
        project_id: requested.project_id.clone(),
        repository_id: requested.repository_connection_id.clone(),
        repository_revision: requested.repository_revision.clone(),
        profile_id: requested.runtime_profile_id.clone(),
        agent: requested.agent.as_ref().map(|a| AgentSelection {
            provider: a.provider.clone(),
            agent: a.agent.clone(),
            model: a.model.clone(),
        }),
        request: requested.request.clone(),
        acceptance_criteria: requested.acceptance_criteria.clone(),
        constraints: requested
            .change_constraints
            .as_ref()
            .map(|c| ChangeConstraints {
                protected_paths: c.protected_paths.clone(),
                allow_test_edits: c.allow_test_edits,
            }),
        coding_turns: requested.coding_turns,
        deadline: requested.deadline.as_ref().map(|d| DeadlineDuration {
            seconds: d.seconds,
            nanos: d.nanos,
        }),
    })
}

pub(super) fn admit_task(
    store: &mut crate::adapters::sqlite::CleanStore,
    command: AdmitTask,
) -> Result<(v1::DevelopmentTask, v1::DevelopmentAttempt), CleanStoreError> {
    let receipt =
        ayni_orchestrator_application::tasks::admit(store, command).map_err(admission_error)?;
    Ok((
        store.development_task(&receipt.task_id)?,
        store.development_attempt(&receipt.attempt_id)?,
    ))
}

pub fn cancel_command(
    key: &str,
    request: &v1::CancelDevelopmentTaskRequest,
) -> ayni_orchestrator_application::tasks::cancellation::CancelTask {
    ayni_orchestrator_application::tasks::cancellation::CancelTask {
        identity: RequestIdentity {
            key: key.into(),
            digest: crate::adapters::sqlite::request_digest("CancelDevelopmentTask", request),
            payload: request.encode_to_vec(),
        },
        task_id: request.development_task_id.clone(),
    }
}

pub(super) fn cancel_task(
    store: &mut crate::adapters::sqlite::CleanStore,
    command: ayni_orchestrator_application::tasks::cancellation::CancelTask,
) -> Result<v1::DevelopmentTask, CleanStoreError> {
    let id = ayni_orchestrator_application::tasks::cancellation::cancel(store, command)
        .map_err(crate::adapters::sqlite::task_cancellation::cancellation_error)?;
    store.development_task(&id)
}

pub fn retry_command(
    key: &str,
    request: &v1::RetryDevelopmentTaskRequest,
) -> ayni_orchestrator_application::tasks::retry::RetryTask {
    ayni_orchestrator_application::tasks::retry::RetryTask {
        task_id: request.development_task_id.clone(),
        identity: RequestIdentity {
            key: key.into(),
            digest: crate::adapters::sqlite::request_digest("RetryDevelopmentTask", request),
            payload: request.encode_to_vec(),
        },
    }
}
pub(super) fn retry_task(
    store: &mut crate::adapters::sqlite::CleanStore,
    command: &ayni_orchestrator_application::tasks::retry::RetryTask,
) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
    let id = ayni_orchestrator_application::tasks::retry::retry(store, command)
        .map_err(crate::adapters::sqlite::task_retry::retry_error)?;
    store.development_attempt(&id)
}
