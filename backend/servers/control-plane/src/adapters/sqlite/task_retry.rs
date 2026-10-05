//! Explicit persistence codec and atomic retry mutation.
use super::{
    CleanStore, CleanStoreError, DEVELOPMENT_SERVICE, Message, action_row, append_event,
    map_constraint, params, persist_task_update, resource_id, timestamp, unix_timestamp, v1,
};
use ayni_orchestrator_application::tasks::{
    RequestIdentity, TimePoint,
    retry::{Repository, RetryError, RetryFacts, RetryTask},
};
pub(crate) fn retry_error(error: RetryError<CleanStoreError>) -> CleanStoreError {
    match error {
        RetryError::Storage(error) => error,
        RetryError::NotActionable => CleanStoreError::TaskNotActionable,
        RetryError::DeadlineExpired => CleanStoreError::DeadlineExpired,
    }
}
impl Repository for CleanStore {
    type Error = CleanStoreError;
    fn replay_retry(&self, identity: &RequestIdentity) -> Result<Option<String>, Self::Error> {
        let Some(action) = action_row(&self.connection, "RetryDevelopmentTask", &identity.key)?
        else {
            return Ok(None);
        };
        if action.request_hash != identity.digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        let id = action.target_id.ok_or(CleanStoreError::CorruptState)?;
        self.development_attempt(&id)?;
        Ok(Some(id))
    }
    fn retry_facts(&self, task_id: &str) -> Result<RetryFacts, Self::Error> {
        let task = self.development_task(task_id)?;
        self.active_project_repository(&task.project_id)?;
        Ok(RetryFacts {
            version: task.resource_version,
            state: super::task_cancellation::task_state(task.state)?,
            attempt_count: task.attempt_count,
            max_attempts: self.max_attempts,
            deadline: task
                .snapshot
                .as_ref()
                .and_then(|s| s.deadline_at.as_ref())
                .map(|d| TimePoint {
                    seconds: d.seconds,
                    nanos: d.nanos,
                }),
        })
    }
    fn retry_time(&self) -> Result<TimePoint, Self::Error> {
        let time = unix_timestamp()?;
        Ok(TimePoint {
            seconds: time.seconds,
            nanos: time.nanos,
        })
    }
    fn commit_retry(
        &mut self,
        command: &RetryTask,
        facts: &RetryFacts,
        time: TimePoint,
    ) -> Result<String, Self::Error> {
        let mut task = self.development_task(&command.task_id)?;
        if task.resource_version != facts.version {
            return Err(CleanStoreError::StaleVersion);
        }
        let now = time.seconds;
        let attempt_number = task.attempt_count.saturating_add(1);
        let attempt_id = resource_id("attempt");
        let attempt = v1::DevelopmentAttempt {
            agent_activity: vec![],
            id: attempt_id.clone(),
            development_task_id: task.id.clone(),
            attempt_number,
            snapshot: task.snapshot.clone(),
            state: v1::DevelopmentAttemptState::Queued as i32,
            requested_input: None,
            failure: None,
            model_usage: Some(v1::AttemptModelUsage {
                collection_status: v1::ModelUsageCollectionStatus::Unavailable as i32,
                ..v1::AttemptModelUsage::default()
            }),
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            completed_at: None,
            resource_version: 1,
        };
        let previous_version = task.resource_version;
        task.state = v1::DevelopmentTaskState::Queued as i32;
        task.active_attempt_id = Some(attempt_id.clone());
        task.attempt_count = attempt_number;
        task.failure = None;
        task.updated_at = Some(timestamp(now));
        task.resource_version = previous_version.saturating_add(1);
        let repository_id = task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.repository.as_ref())
            .map(|repository| repository.repository_connection_id.clone())
            .ok_or(CleanStoreError::CorruptState)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('RetryDevelopmentTask',?1,?2,?3,'completed','development_attempt',?4,?5,?5)",
                params![command.identity.key, command.identity.digest, command.identity.payload, attempt_id, now],
            )
            .map_err(map_constraint)?;
        persist_task_update(&transaction, &task, previous_version)?;
        let cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_attempt",
            &attempt.id,
            1,
            Some(&repository_id),
            Some(&task.id),
            &attempt,
        )?;
        transaction
            .execute(
                "INSERT INTO development_attempts(
                    id,development_task_id,attempt_number,state,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    attempt.id,
                    task.id,
                    attempt.attempt_number,
                    attempt.state,
                    attempt.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt.id)
    }
}
