use super::{
    CleanStore, CleanStoreError, DEVELOPMENT_SERVICE, MAX_RECONCILE_BATCH, PROFILE_SERVICE, Page,
    Timestamp, bounded_failure, collect_bounded_payloads, decode, encode_filter,
    equivalent_failure, get_payload, is_terminal_attempt, params, persist_attempt_update,
    persist_task_update, query_payloads, task_state_for_attempt, timestamp, unix_seconds,
    unix_timestamp, v1, valid_attempt_transition,
};

impl CleanStore {
    #[cfg(test)]
    pub(crate) fn install_repository_automation(
        &mut self,
        key: &str,
        request: &v1::InstallRepositoryAutomationRequest,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        let command = crate::adapters::grpc::automation_command(key, request);
        let id = ayni_orchestrator_application::automation::install(
            self,
            &crate::profiles::ayni::automation::ManagedAutomation,
            &command,
        )
        .map_err(super::automation_admission::admission_error)?;
        self.automation_installation(&id)
    }

    pub(crate) fn automation_installation(
        &self,
        id: &str,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM automation_installations WHERE id=?1",
            id,
            "automation_installation",
        )
    }

    pub(crate) fn list_automation_installations(
        &mut self,
        profile_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::AutomationInstallation>, CleanStoreError> {
        self.runtime_profile(profile_id)?;
        let filter = encode_filter(&[profile_id]);
        self.page_messages(
            "ListAutomationInstallations",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "automation_installations",
                    "runtime_profile_id",
                    None,
                    Some(profile_id),
                )
            },
            PROFILE_SERVICE,
        )
    }

    pub(crate) fn reconcile_runtime_profile(
        &mut self,
        key: &str,
        request: &v1::ReconcileRuntimeProfileRequest,
    ) -> Result<v1::RuntimeProfile, CleanStoreError> {
        let profile = self.runtime_profile(&request.runtime_profile_id)?;
        let _action = self.begin_external_action(
            "ReconcileRuntimeProfile",
            key,
            request,
            "runtime_profile",
            &profile.id,
        )?;
        Ok(profile)
    }

    #[cfg(test)]
    pub(crate) fn create_development_task(
        &mut self,
        key: &str,
        request: &v1::CreateDevelopmentTaskRequest,
    ) -> Result<(v1::DevelopmentTask, v1::DevelopmentAttempt), CleanStoreError> {
        let command = crate::adapters::grpc::task_command(key, request)?;
        let receipt = ayni_orchestrator_application::tasks::admit(self, command)
            .map_err(super::task_admission::admission_error)?;
        Ok((
            self.development_task(&receipt.task_id)?,
            self.development_attempt(&receipt.attempt_id)?,
        ))
    }

    pub(crate) fn development_task(
        &self,
        id: &str,
    ) -> Result<v1::DevelopmentTask, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM development_tasks WHERE id=?1",
            id,
            "development_task",
        )
    }

    pub(crate) fn list_development_tasks(
        &mut self,
        project_id: Option<&str>,
        repository_connection_id: Option<&str>,
        runtime_profile_id: Option<&str>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::DevelopmentTask>, CleanStoreError> {
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            project_id.unwrap_or(""),
            repository_connection_id.unwrap_or(""),
            runtime_profile_id.unwrap_or(""),
            &state_text,
        ]);
        self.page_messages(
            "ListDevelopmentTasks",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM development_tasks
                         WHERE (?1 IS NULL OR repository_connection_id=?1)
                           AND (?2 IS NULL OR runtime_profile_id=?2)
                           AND (?3 IS NULL OR state=?3)
                           AND (?4 IS NULL OR project_id=?4)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![
                        repository_connection_id,
                        runtime_profile_id,
                        state,
                        project_id
                    ])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            DEVELOPMENT_SERVICE,
        )
    }

    pub(crate) fn development_attempt(
        &self,
        id: &str,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM development_attempts WHERE id=?1",
            id,
            "development_attempt",
        )
    }

    pub(crate) fn list_development_attempts(
        &mut self,
        task_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::DevelopmentAttempt>, CleanStoreError> {
        self.development_task(task_id)?;
        let filter = encode_filter(&[task_id]);
        self.page_messages(
            "ListDevelopmentAttempts",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "development_attempts",
                    "development_task_id",
                    None,
                    Some(task_id),
                )
            },
            DEVELOPMENT_SERVICE,
        )
    }

    #[cfg(test)]
    pub(crate) fn cancel_development_task(
        &mut self,
        key: &str,
        request: &v1::CancelDevelopmentTaskRequest,
    ) -> Result<v1::DevelopmentTask, CleanStoreError> {
        let command = crate::adapters::grpc::cancel_command(key, request);
        let id = ayni_orchestrator_application::tasks::cancellation::cancel(self, command)
            .map_err(super::task_cancellation::cancellation_error)?;
        self.development_task(&id)
    }

    #[cfg(test)]
    pub(crate) fn retry_development_task(
        &mut self,
        key: &str,
        request: &v1::RetryDevelopmentTaskRequest,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let command = crate::adapters::grpc::retry_command(key, request);
        let id = ayni_orchestrator_application::tasks::retry::retry(self, &command)
            .map_err(super::task_retry::retry_error)?;
        self.development_attempt(&id)
    }

    pub(crate) fn reconcile_once(&mut self) -> Result<(), CleanStoreError> {
        let _ = self.expire_deadlines(&unix_timestamp()?)?;
        self.prune_page_snapshots()?;
        Ok(())
    }

    pub(crate) fn expire_deadlines(&mut self, now: &Timestamp) -> Result<usize, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT payload FROM development_tasks
                 WHERE state IN (?1,?2,?3) AND deadline_at_seconds IS NOT NULL
                   AND (deadline_at_seconds<?4
                        OR (deadline_at_seconds=?4 AND deadline_at_nanos<=?5))
                 ORDER BY deadline_at_seconds,deadline_at_nanos,id LIMIT ?6",
            )
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query(params![
                v1::DevelopmentTaskState::Queued as i32,
                v1::DevelopmentTaskState::WaitingForRuntime as i32,
                v1::DevelopmentTaskState::Running as i32,
                now.seconds,
                now.nanos,
                i64::try_from(MAX_RECONCILE_BATCH).map_err(|_| CleanStoreError::CorruptState)?,
            ])
            .map_err(CleanStoreError::Database)?;
        let tasks = collect_bounded_payloads(&mut rows)?;
        drop(rows);
        drop(statement);
        let mut expired = 0_usize;
        for payload in tasks {
            let mut task: v1::DevelopmentTask = decode(&payload)?;
            let mut attempt = task
                .active_attempt_id
                .as_deref()
                .map(|id| self.development_attempt(id))
                .transpose()?;
            let plan = ayni_orchestrator_application::tasks::deadline::expiration(
                super::task_cancellation::task_state(task.state)?,
                task.snapshot
                    .as_ref()
                    .and_then(|s| s.deadline_at.as_ref())
                    .map(|d| ayni_orchestrator_application::tasks::TimePoint {
                        seconds: d.seconds,
                        nanos: d.nanos,
                    }),
                ayni_orchestrator_application::tasks::TimePoint {
                    seconds: now.seconds,
                    nanos: now.nanos,
                },
                attempt
                    .as_ref()
                    .map(|a| super::task_cancellation::attempt_state(a.state))
                    .transpose()?,
            );
            let Some(plan) = plan else {
                continue;
            };
            let previous_task_version = task.resource_version;
            task.state = v1::DevelopmentTaskState::DeadlineExceeded as i32;
            task.failure = Some(v1::ResourceFailure {
                reason: v1::ErrorReason::TaskDeadlineExpired as i32,
                message: "the development task deadline expired".to_owned(),
                remediation: None,
                observed_at: Some(now.to_owned()),
            });
            task.updated_at = Some(now.to_owned());
            task.resource_version = task.resource_version.saturating_add(1);
            if let Some(value) = attempt.as_mut()
                && plan.expire_attempt
            {
                value.state = v1::DevelopmentAttemptState::DeadlineExceeded as i32;
                value.failure.clone_from(&task.failure);
                value.updated_at = Some(now.to_owned());
                value.completed_at = Some(now.to_owned());
                value.resource_version = value.resource_version.saturating_add(1);
            }
            let transaction = self
                .connection
                .transaction()
                .map_err(CleanStoreError::Database)?;
            persist_task_update(&transaction, &task, previous_task_version)?;
            if let Some(value) = attempt.filter(|_| plan.expire_attempt) {
                persist_attempt_update(
                    &transaction,
                    &value,
                    value.resource_version.saturating_sub(1),
                    &task,
                )?;
            }
            transaction.commit().map_err(CleanStoreError::Database)?;
            expired = expired.saturating_add(1);
        }
        Ok(expired)
    }

    pub(crate) fn transition_attempt(
        &mut self,
        attempt_id: &str,
        expected_version: u64,
        new_state: v1::DevelopmentAttemptState,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let mut attempt = self.development_attempt(attempt_id)?;
        let current = v1::DevelopmentAttemptState::try_from(attempt.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        let failure = bounded_failure(failure);
        if current == new_state
            && equivalent_failure(attempt.failure.as_ref(), failure.as_ref())
            && (attempt.resource_version == expected_version
                || attempt.resource_version == expected_version.saturating_add(1))
        {
            return Ok(attempt);
        }
        if attempt.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if !valid_attempt_transition(current, new_state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let now = unix_seconds()?;
        attempt.state = new_state as i32;
        attempt.failure = failure;
        attempt.updated_at = Some(timestamp(now));
        attempt.resource_version = attempt.resource_version.saturating_add(1);
        if is_terminal_attempt(attempt.state) {
            attempt.completed_at = Some(timestamp(now));
        }
        let mut task = self.development_task(&attempt.development_task_id)?;
        let previous_task_version = task.resource_version;
        if task.active_attempt_id.as_deref() == Some(attempt_id) {
            task.state = task_state_for_attempt(new_state) as i32;
            task.failure.clone_from(&attempt.failure);
            task.updated_at = Some(timestamp(now));
            task.resource_version = task.resource_version.saturating_add(1);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, expected_version, &task)?;
        if task.resource_version != previous_task_version {
            persist_task_update(&transaction, &task, previous_task_version)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(attempt)
    }
}
