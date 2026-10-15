//! Atomic persistence and codecs for application cancellation decisions.
use super::{
    CleanStore, CleanStoreError, action_row, map_constraint, params, persist_attempt_update,
    persist_task_update, timestamp, unix_seconds, v1,
};
use ayni_orchestrator_application::tasks::{
    RequestIdentity,
    cancellation::{
        AttemptTarget, CancellationError, CancellationPlan, CancellationRepository,
        CancellationTarget,
    },
    lifecycle::{AttemptState, TaskState},
};

pub(crate) fn cancellation_error(error: CancellationError<CleanStoreError>) -> CleanStoreError {
    match error {
        CancellationError::Storage(error) => error,
        CancellationError::NotActionable => CleanStoreError::TaskNotActionable,
    }
}

pub(super) fn task_state(value: i32) -> Result<TaskState, CleanStoreError> {
    use v1::DevelopmentTaskState as Wire;
    Ok(
        match Wire::try_from(value).map_err(|_| CleanStoreError::CorruptState)? {
            Wire::Queued => TaskState::Queued,
            Wire::WaitingForRuntime => TaskState::WaitingForRuntime,
            Wire::Running => TaskState::Running,
            Wire::Succeeded => TaskState::Succeeded,
            Wire::Failed => TaskState::Failed,
            Wire::Cancelled => TaskState::Cancelled,
            Wire::DeadlineExceeded => TaskState::DeadlineExceeded,
            Wire::Unsupported => TaskState::Unsupported,
            Wire::Unspecified => return Err(CleanStoreError::CorruptState),
        },
    )
}

pub(super) fn attempt_state(value: i32) -> Result<AttemptState, CleanStoreError> {
    use v1::DevelopmentAttemptState as Wire;
    Ok(
        match Wire::try_from(value).map_err(|_| CleanStoreError::CorruptState)? {
            Wire::Queued => AttemptState::Queued,
            Wire::Running => AttemptState::Running,
            Wire::WaitingForInput => AttemptState::WaitingForInput,
            Wire::Succeeded => AttemptState::Succeeded,
            Wire::Failed => AttemptState::Failed,
            Wire::Cancelled => AttemptState::Cancelled,
            Wire::DeadlineExceeded => AttemptState::DeadlineExceeded,
            Wire::Unsupported => AttemptState::Unsupported,
            Wire::Unspecified => return Err(CleanStoreError::CorruptState),
        },
    )
}

impl CancellationRepository for CleanStore {
    type Error = CleanStoreError;

    fn cancellation_replay(
        &self,
        identity: &RequestIdentity,
    ) -> Result<Option<String>, Self::Error> {
        let Some(action) = action_row(&self.connection, "CancelDevelopmentTask", &identity.key)?
        else {
            return Ok(None);
        };
        if action.request_hash != identity.digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        Ok(Some(action.target_id.ok_or(CleanStoreError::CorruptState)?))
    }

    fn cancellation_target(&self, task_id: &str) -> Result<CancellationTarget, Self::Error> {
        let task = self.development_task(task_id)?;
        let attempt = task
            .active_attempt_id
            .as_deref()
            .map(|id| self.development_attempt(id))
            .transpose()?
            .map(|a| {
                Ok(AttemptTarget {
                    id: a.id,
                    state: attempt_state(a.state)?,
                    version: a.resource_version,
                })
            })
            .transpose()?;
        Ok(CancellationTarget {
            task_id: task.id,
            state: task_state(task.state)?,
            version: task.resource_version,
            attempt,
        })
    }

    fn commit_cancellation(
        &mut self,
        identity: RequestIdentity,
        plan: CancellationPlan,
    ) -> Result<String, Self::Error> {
        let mut task = self.development_task(&plan.target.task_id)?;
        if task.resource_version != plan.target.version {
            return Err(CleanStoreError::InvalidTransition);
        }
        let now = unix_seconds()?;
        let attempt = if plan.cancel_attempt {
            let target = plan
                .target
                .attempt
                .as_ref()
                .ok_or(CleanStoreError::CorruptState)?;
            let mut value = self.development_attempt(&target.id)?;
            if value.resource_version != target.version {
                return Err(CleanStoreError::InvalidTransition);
            }
            value.state = v1::DevelopmentAttemptState::Cancelled as i32;
            value.updated_at = Some(timestamp(now));
            value.completed_at = Some(timestamp(now));
            value.failure = None;
            value.resource_version = value.resource_version.saturating_add(1);
            Some(value)
        } else {
            None
        };
        let changed = plan.target.state != TaskState::Cancelled;
        if changed {
            task.state = v1::DevelopmentTaskState::Cancelled as i32;
            task.failure = None;
            task.updated_at = Some(timestamp(now));
            task.resource_version = task.resource_version.saturating_add(1);
        }
        let transaction = self
            .connection
            .savepoint()
            .map_err(CleanStoreError::Database)?;
        transaction.execute(
            "INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at)
             VALUES ('CancelDevelopmentTask',?1,?2,?3,'completed','development_task',?4,?5,?5)",
            params![identity.key, identity.digest, identity.payload, task.id, now],
        ).map_err(map_constraint)?;
        if changed {
            persist_task_update(&transaction, &task, plan.target.version)?;
        }
        if let Some(value) = attempt {
            let previous = plan
                .target
                .attempt
                .as_ref()
                .ok_or(CleanStoreError::CorruptState)?
                .version;
            persist_attempt_update(&transaction, &value, previous, &task)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(task.id)
    }
}
