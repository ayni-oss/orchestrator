use super::{
    CleanStore, CleanStoreError, collect_bounded_payloads, decode, params, persist_attempt_update,
    timestamp, unix_seconds, v1, valid_model_usage,
};

impl CleanStore {
    pub(crate) const fn configure_worker_limits(
        &mut self,
        coding_turns: u32,
        quality_repair_limit: u32,
    ) {
        self.coding_turns = coding_turns;
        self.quality_repair_limit = quality_repair_limit;
    }

    pub(crate) fn coding_activity(
        &mut self,
        attempt_id: &str,
        activity: Vec<v1::AgentActivity>,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let mut attempt = self.development_attempt(attempt_id)?;
        if attempt.agent_activity == activity {
            return Ok(attempt);
        }
        let task = self.development_task(&attempt.development_task_id)?;
        let previous = attempt.resource_version;
        attempt.agent_activity = activity;
        attempt.resource_version += 1;
        attempt.updated_at = Some(timestamp(unix_seconds()?));
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, previous, &task)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt)
    }

    pub(crate) fn coding_attempts(
        &self,
        profile_id: &str,
    ) -> Result<Vec<v1::DevelopmentAttempt>, CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT a.payload FROM development_attempts a JOIN development_tasks t ON t.id=a.development_task_id WHERE t.runtime_profile_id=?1 AND NOT EXISTS (SELECT 1 FROM reconciliation_cursors c WHERE c.name='coding-done:'||a.id) ORDER BY a.id LIMIT 128").map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query([profile_id])
            .map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)?
            .into_iter()
            .map(|bytes| decode(&bytes))
            .collect()
    }

    pub(crate) fn coding_cleanup_complete(
        &mut self,
        attempt_id: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("INSERT OR IGNORE INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES (?1,'{}',1,?2)", params![format!("coding-done:{attempt_id}"),unix_seconds()?]).map_err(CleanStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn coding_details(
        &mut self,
        attempt_id: &str,
        requested_input: Option<String>,
        usage: v1::AttemptModelUsage,
    ) -> Result<(), CleanStoreError> {
        if requested_input
            .as_ref()
            .is_some_and(|value| value.len() > 1024)
            || !valid_model_usage(Some(&usage))
        {
            return Err(CleanStoreError::BoundExceeded);
        }
        let mut attempt = self.development_attempt(attempt_id)?;
        let task = self.development_task(&attempt.development_task_id)?;
        if attempt.requested_input == requested_input
            && attempt.model_usage.as_ref() == Some(&usage)
        {
            return Ok(());
        }
        let previous = attempt.resource_version;
        attempt.requested_input = requested_input;
        attempt.model_usage = Some(usage);
        attempt.resource_version += 1;
        attempt.updated_at = Some(timestamp(unix_seconds()?));
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, previous, &task)?;
        transaction.commit().map_err(CleanStoreError::Database)
    }
}

impl CleanStore {
    pub(crate) fn coding_result_exists(&self, id: &str) -> Result<bool, CleanStoreError> {
        Ok(self.development_attempt(id)?.result.is_some())
    }
    fn validate_worker_result(
        &self,
        attempt: &v1::DevelopmentAttempt,
        value: &v1::WorkerResult,
    ) -> Result<(), CleanStoreError> {
        let limits = attempt
            .snapshot
            .as_ref()
            .and_then(|s| s.limits.as_ref())
            .ok_or(CleanStoreError::CorruptState)?;
        if value.coding_turns_consumed > limits.coding_turns
            || value.quality_round > limits.quality_repair_limit
            || value.changed_paths.len() > 256
            || value.changed_paths.iter().any(|p| {
                p.is_empty()
                    || p.starts_with('/')
                    || p.split('/').any(|c| c == ".." || c.is_empty())
            })
            || value
                .commit
                .as_ref()
                .is_some_and(|s| !super::valid_git_revision(s))
        {
            return Err(CleanStoreError::BoundExceeded);
        }
        let quality = value
            .quality
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        validate_worker_quality(quality, value.commit.as_ref())?;
        if let Some(ref receipt) = value.pull_request {
            let snapshot = attempt
                .snapshot
                .as_ref()
                .ok_or(CleanStoreError::CorruptState)?;
            let source = snapshot
                .repository
                .as_ref()
                .ok_or(CleanStoreError::CorruptState)?;
            let repository = self.repository_connection(&source.repository_connection_id)?;
            let Some(v1::repository_provider::Provider::Github(github)) =
                repository.provider.and_then(|p| p.provider)
            else {
                return Err(CleanStoreError::CorruptState);
            };
            if receipt.repository != format!("{}/{}", github.owner, github.name)
                || receipt.base_branch != snapshot.publication_base_branch
                || receipt.branch != format!("task/{}", attempt.id.replace(':', "-"))
                || receipt.url
                    != format!(
                        "https://github.com/{}/pull/{}",
                        receipt.repository, receipt.number
                    )
            {
                return Err(CleanStoreError::CorruptState);
            }
        }
        if value.publication == v1::PublicationOutcome::Published as i32
            && value.pull_request.as_ref().is_none_or(|p| {
                !p.draft || Some(&p.head_commit) != value.commit.as_ref() || p.number == 0
            })
        {
            return Err(CleanStoreError::CorruptState);
        }
        Ok(())
    }
    pub(crate) fn collect_worker(
        &mut self,
        id: &str,
        progress: Option<v1::WorkerProgress>,
        result: Option<v1::WorkerResult>,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let mut attempt = self.development_attempt(id)?;
        let mut task = self.development_task(&attempt.development_task_id)?;
        if [
            v1::DevelopmentAttemptState::Cancelled as i32,
            v1::DevelopmentAttemptState::DeadlineExceeded as i32,
        ]
        .contains(&attempt.state)
        {
            return Ok(attempt);
        }
        if let Some(ref value) = result {
            self.validate_worker_result(&attempt, value)?;
            if attempt.result.as_ref() == Some(value) && progress.is_none() {
                return Ok(attempt);
            }
        }
        if let Some(ref progress) = progress {
            validate_worker_progress(&attempt, progress)?;
        }
        if result.is_none() && progress.as_ref() == attempt.progress.as_ref() {
            return Ok(attempt);
        }
        let previous = attempt.resource_version;
        if let Some(progress) = progress {
            attempt.progress = Some(progress);
        }
        if let Some(result) = result {
            attempt.requested_input.clone_from(&result.requested_input);
            attempt.model_usage.clone_from(&result.model_usage);
            attempt.pull_request.clone_from(&result.pull_request);
            attempt.state = match v1::ExecutionOutcome::try_from(result.execution)
                .map_err(|_| CleanStoreError::CorruptState)?
            {
                v1::ExecutionOutcome::Succeeded | v1::ExecutionOutcome::NoChanges => {
                    v1::DevelopmentAttemptState::Succeeded
                }
                v1::ExecutionOutcome::InputRequired => v1::DevelopmentAttemptState::WaitingForInput,
                v1::ExecutionOutcome::Cancelled => v1::DevelopmentAttemptState::Cancelled,
                v1::ExecutionOutcome::DeadlineExceeded => {
                    v1::DevelopmentAttemptState::DeadlineExceeded
                }
                _ => v1::DevelopmentAttemptState::Failed,
            } as i32;
            attempt.completed_at = Some(timestamp(unix_seconds()?));
            attempt.result = Some(result);
        }
        attempt.resource_version += 1;
        attempt.updated_at = Some(timestamp(unix_seconds()?));
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, previous, &task)?;
        if task.active_attempt_id.as_deref() == Some(id) {
            let previous = task.resource_version;
            task.state = super::task_state_for_attempt(
                v1::DevelopmentAttemptState::try_from(attempt.state)
                    .map_err(|_| CleanStoreError::CorruptState)?,
            ) as i32;
            task.resource_version += 1;
            task.updated_at = attempt.updated_at;
            super::persist_task_update(&transaction, &task, previous)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt)
    }
    pub(crate) fn retry_task_publication(
        &mut self,
        key: &str,
        request: &v1::RetryTaskPublicationRequest,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let mut task = self.development_task(&request.development_task_id)?;
        let id = task
            .active_attempt_id
            .clone()
            .ok_or(CleanStoreError::TaskNotActionable)?;
        let mut attempt = self.development_attempt(&id)?;
        let hash = super::request_digest("RetryTaskPublication", request);
        if let Some(action) = super::action_row(&self.connection, "RetryTaskPublication", key)? {
            if action.request_hash != hash {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return Ok(attempt);
        }
        let result = attempt
            .result
            .as_ref()
            .ok_or(CleanStoreError::TaskNotActionable)?;
        if result.commit.is_none()
            || ![
                v1::PublicationOutcome::Failed as i32,
                v1::PublicationOutcome::Pending as i32,
            ]
            .contains(&result.publication)
            || [
                v1::DevelopmentAttemptState::Cancelled as i32,
                v1::DevelopmentAttemptState::DeadlineExceeded as i32,
            ]
            .contains(&attempt.state)
        {
            return Err(CleanStoreError::TaskNotActionable);
        }
        if attempt
            .snapshot
            .as_ref()
            .and_then(|s| s.limits.as_ref())
            .and_then(|l| l.deadline_at.as_ref())
            .is_none_or(|d| d.seconds <= unix_seconds().unwrap_or(i64::MAX))
        {
            return Err(CleanStoreError::DeadlineExpired);
        }
        attempt.publication_only = true;
        let previous = attempt.resource_version;
        attempt.state = v1::DevelopmentAttemptState::Queued as i32;
        attempt.resource_version += 1;
        attempt.updated_at = Some(timestamp(unix_seconds()?));
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, previous, &task)?;
        let previous = task.resource_version;
        task.state = v1::DevelopmentTaskState::Queued as i32;
        task.resource_version += 1;
        super::persist_task_update(&transaction, &task, previous)?;
        transaction
            .execute(
                "DELETE FROM reconciliation_cursors WHERE name=?1",
                [format!("coding-done:{id}")],
            )
            .map_err(CleanStoreError::Database)?;
        transaction.execute("INSERT INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES ('RetryTaskPublication',?1,?2,?3,'completed','development_attempt',?4,?5,?5)",params![key,hash,prost::Message::encode_to_vec(request),id,unix_seconds()?]).map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt)
    }
}

fn validate_worker_progress(
    attempt: &v1::DevelopmentAttempt,
    progress: &v1::WorkerProgress,
) -> Result<(), CleanStoreError> {
    let limits = attempt
        .snapshot
        .as_ref()
        .and_then(|s| s.limits.as_ref())
        .ok_or(CleanStoreError::CorruptState)?;
    if progress.coding_turns_consumed > limits.coding_turns
        || progress.quality_round > limits.quality_repair_limit
        || attempt.progress.as_ref().is_some_and(|old| {
            old.coding_turns_consumed > progress.coding_turns_consumed
                || old.quality_round > progress.quality_round
        })
    {
        return Err(CleanStoreError::CorruptState);
    }
    Ok(())
}

fn validate_worker_quality(
    quality: &v1::QualityResult,
    commit: Option<&String>,
) -> Result<(), CleanStoreError> {
    if quality.rows.len() > 32
        || quality.findings.len() > 32
        || quality.findings.iter().any(|s| s.len() > 4096)
        || (quality.outcome == v1::QualityOutcome::Passed as i32
            && quality.evidence.as_ref().is_none_or(|e| {
                e.artifact_path != "evidence/signals.json"
                    || !super::payloads::valid_digest(&e.signals_digest)
                    || Some(&e.commit) != commit
            }))
    {
        return Err(CleanStoreError::CorruptState);
    }
    Ok(())
}
