use crate::{github_setup, onboarding_templates};

impl CleanStore {
    pub(crate) fn install_repository_automation(
        &mut self,
        key: &str,
        request: &v1::InstallRepositoryAutomationRequest,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        let digest = request_digest("InstallRepositoryAutomation", request);
        if let Some(existing) = action_row(&self.connection, "InstallRepositoryAutomation", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self.automation_installation(
                &existing.target_id.ok_or(CleanStoreError::CorruptState)?,
            );
        }
        let profile = self.runtime_profile(&request.runtime_profile_id)?;
        let repository = self.repository_connection(&profile.repository_connection_id)?;
        let github = repository
            .provider
            .and_then(|provider| provider.provider)
            .map(|provider| match provider {
                v1::repository_provider::Provider::Github(value) => value,
            })
            .ok_or(CleanStoreError::CorruptState)?;
        let revision_id = profile
            .active_revision_id
            .ok_or(CleanStoreError::CorruptState)?;
        let revision = self.runtime_profile_revision(&revision_id)?;
        let version = match revision.configuration {
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(value)) => {
                value.workflow_template_version
            }
            _ => return Err(CleanStoreError::InvalidTransition),
        };
        let workflows = onboarding_templates::render(&version, &repository.default_branch)
            .map_err(|_| CleanStoreError::UnsupportedWorkflowTemplate(version.clone()))?;
        github_setup::setup_pull_request(&profile.id, &repository.default_branch, workflows)
            .map_err(|_| CleanStoreError::CorruptState)?;
        let now = unix_seconds()?;
        let id = resource_id("automation");
        let installation = v1::AutomationInstallation {
            id: id.clone(),
            runtime_profile_id: profile.id.clone(),
            version: version.clone(),
            state: v1::AutomationInstallationState::SetupRequired as i32,
            provider: Some(v1::automation_installation::Provider::Github(
                v1::GithubAutomationInstallation {
                    installation_id: github.installation_id,
                    setup_pull_request_number: None,
                    environment_workflow_path: onboarding_templates::ENVIRONMENT_WORKFLOW_PATH
                        .to_owned(),
                    assessment_workflow_path: onboarding_templates::ASSESSMENT_WORKFLOW_PATH
                        .to_owned(),
                },
            )),
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('InstallRepositoryAutomation',?1,?2,?3,'pending','automation_installation',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "automation_installation",
            &id,
            1,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &installation,
        )?;
        transaction
            .execute(
                "INSERT INTO automation_installations(
                    id,runtime_profile_id,state,version,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,1,?6)",
                params![
                    id,
                    profile.id,
                    installation.state,
                    version,
                    installation.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(installation)
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

    pub(crate) fn transition_automation_installation(
        &mut self,
        id: &str,
        expected_version: u64,
        state: v1::AutomationInstallationState,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::AutomationInstallation, CleanStoreError> {
        let mut resource = self.automation_installation(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::AutomationInstallationState::try_from(resource.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_automation_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let profile = self.runtime_profile(&resource.runtime_profile_id)?;
        resource.state = state as i32;
        resource.failure = bounded_failure(failure);
        resource.updated_at = Some(timestamp(unix_seconds()?));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            PROFILE_SERVICE,
            "automation_installation",
            &resource.id,
            resource.resource_version,
            Some(&profile.repository_connection_id),
            Some(&profile.id),
            &resource,
        )?;
        let changed = transaction.execute(
            "UPDATE automation_installations SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
            params![resource.state, resource.encode_to_vec(),
                i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,
                i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?, resource.id,
                i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?],
        ).map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
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

    pub(crate) fn create_development_task(
        &mut self,
        key: &str,
        request: &v1::CreateDevelopmentTaskRequest,
    ) -> Result<(v1::DevelopmentTask, v1::DevelopmentAttempt), CleanStoreError> {
        let digest = request_digest("CreateDevelopmentTask", request);
        if let Some(existing) = action_row(&self.connection, "CreateDevelopmentTask", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            let task =
                self.development_task(&existing.target_id.ok_or(CleanStoreError::CorruptState)?)?;
            let initial_attempt: v1::DevelopmentAttempt = get_payload(
                &self.connection,
                "SELECT payload FROM development_attempts
                 WHERE development_task_id=?1 AND attempt_number=1",
                &task.id,
                "development_attempt",
            )?;
            return Ok((task, initial_attempt));
        }
        let requested = request
            .request
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?;
        let repository = self.repository_connection(&requested.repository_connection_id)?;
        let repository_snapshot =
            self.repository_snapshot_for_revision(&repository.id, &requested.repository_revision)?;
        let profile = self.runtime_profile(&requested.runtime_profile_id)?;
        if profile.repository_connection_id != repository.id {
            return Err(CleanStoreError::ParentMismatch);
        }
        if profile.state != v1::RuntimeProfileState::Ready as i32
            || profile.support != v1::RuntimeProfileSupport::Operational as i32
        {
            return Err(CleanStoreError::ProfileNotReady);
        }
        let revision_id = profile
            .active_revision_id
            .clone()
            .ok_or(CleanStoreError::ProfileNotReady)?;
        let artifact_id = profile
            .active_runtime_artifact_id
            .clone()
            .ok_or(CleanStoreError::ProfileNotReady)?;
        let revision = self.runtime_profile_revision(&revision_id)?;
        let artifact = self.runtime_artifact(&artifact_id)?;
        if artifact.runtime_profile_revision_id != revision.id
            || artifact.status != v1::RuntimeArtifactStatus::Verified as i32
        {
            return Err(CleanStoreError::ProfileNotReady);
        }
        let now_timestamp = unix_timestamp()?;
        let now = now_timestamp.seconds;
        let deadline_at = requested
            .deadline
            .as_ref()
            .map(|duration| add_duration(&now_timestamp, duration))
            .transpose()?;
        let snapshot = v1::DevelopmentSnapshot {
            repository: Some(v1::FrozenRepository {
                repository_connection_id: repository.id.clone(),
                repository_snapshot_id: repository_snapshot.id,
                revision: repository_snapshot.revision,
            }),
            runtime: Some(v1::FrozenRuntime {
                runtime_profile_id: profile.id.clone(),
                runtime_profile_revision_id: revision.id,
                profile_kind: profile.kind,
                runtime_artifact_id: artifact.id,
                oci_repository: artifact.oci_repository,
                resolved_digest: artifact.resolved_digest,
                platform: artifact.platform,
            }),
            agent: requested.agent.clone(),
            request: requested.request.clone(),
            acceptance_criteria: requested.acceptance_criteria.clone(),
            change_constraints: requested.change_constraints.clone(),
            deadline_at,
        };
        let task_id = resource_id("task");
        let attempt_id = resource_id("attempt");
        let task = v1::DevelopmentTask {
            id: task_id.clone(),
            snapshot: Some(snapshot.clone()),
            state: v1::DevelopmentTaskState::Queued as i32,
            active_attempt_id: Some(attempt_id.clone()),
            attempt_count: 1,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let attempt = v1::DevelopmentAttempt {
            id: attempt_id.clone(),
            development_task_id: task_id.clone(),
            attempt_number: 1,
            snapshot: Some(snapshot),
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
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CreateDevelopmentTask',?1,?2,?3,'completed','development_task',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), task_id, now],
            )
            .map_err(map_constraint)?;
        let task_cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_task",
            &task_id,
            1,
            Some(&repository.id),
            Some(&task_id),
            &task,
        )?;
        transaction
            .execute(
                "INSERT INTO development_tasks(
                    id,repository_connection_id,runtime_profile_id,state,active_attempt_id,
                    deadline_at_seconds,deadline_at_nanos,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9)",
                params![
                    task_id,
                    repository.id,
                    profile.id,
                    task.state,
                    attempt_id,
                    task.snapshot
                        .as_ref()
                        .and_then(|value| value.deadline_at.as_ref())
                        .map(|value| value.seconds),
                    task.snapshot
                        .as_ref()
                        .and_then(|value| value.deadline_at.as_ref())
                        .map(|value| value.nanos),
                    task.encode_to_vec(),
                    i64::try_from(task_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        let attempt_cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_attempt",
            &attempt_id,
            1,
            Some(&repository.id),
            Some(&task_id),
            &attempt,
        )?;
        transaction
            .execute(
                "INSERT INTO development_attempts(
                    id,development_task_id,attempt_number,state,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,1,?3,?4,1,?5)",
                params![
                    attempt_id,
                    task_id,
                    attempt.state,
                    attempt.encode_to_vec(),
                    i64::try_from(attempt_cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok((task, attempt))
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
        repository_connection_id: Option<&str>,
        runtime_profile_id: Option<&str>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::DevelopmentTask>, CleanStoreError> {
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
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
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![repository_connection_id, runtime_profile_id, state])
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

    pub(crate) fn cancel_development_task(
        &mut self,
        key: &str,
        request: &v1::CancelDevelopmentTaskRequest,
    ) -> Result<v1::DevelopmentTask, CleanStoreError> {
        let digest = request_digest("CancelDevelopmentTask", request);
        if let Some(existing) = action_row(&self.connection, "CancelDevelopmentTask", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self
                .development_task(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let mut task = self.development_task(&request.development_task_id)?;
        if is_terminal_task(task.state) {
            if task.state == v1::DevelopmentTaskState::Cancelled as i32 {
                self.record_completed_action(
                    "CancelDevelopmentTask",
                    key,
                    request,
                    "development_task",
                    &task.id,
                )?;
                return Ok(task);
            }
            return Err(CleanStoreError::TaskNotActionable);
        }
        let now = unix_seconds()?;
        task.state = v1::DevelopmentTaskState::Cancelled as i32;
        task.failure = None;
        task.updated_at = Some(timestamp(now));
        let previous_version = task.resource_version;
        task.resource_version = previous_version.saturating_add(1);
        let mut attempt = task
            .active_attempt_id
            .as_deref()
            .map(|id| self.development_attempt(id))
            .transpose()?;
        if let Some(value) = attempt.as_mut()
            && !is_terminal_attempt(value.state)
        {
            value.state = v1::DevelopmentAttemptState::Cancelled as i32;
            value.updated_at = Some(timestamp(now));
            value.completed_at = Some(timestamp(now));
            value.failure = None;
            value.resource_version = value.resource_version.saturating_add(1);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('CancelDevelopmentTask',?1,?2,?3,'completed','development_task',?4,?5,?5)",
                params![key, digest, request.encode_to_vec(), task.id, now],
            )
            .map_err(map_constraint)?;
        persist_task_update(&transaction, &task, previous_version)?;
        if let Some(value) = attempt {
            persist_attempt_update(
                &transaction,
                &value,
                value.resource_version.saturating_sub(1),
                &task,
            )?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(task)
    }

    pub(crate) fn retry_development_task(
        &mut self,
        key: &str,
        request: &v1::RetryDevelopmentTaskRequest,
    ) -> Result<v1::DevelopmentAttempt, CleanStoreError> {
        let digest = request_digest("RetryDevelopmentTask", request);
        if let Some(existing) = action_row(&self.connection, "RetryDevelopmentTask", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self
                .development_attempt(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let mut task = self.development_task(&request.development_task_id)?;
        if !matches!(
            v1::DevelopmentTaskState::try_from(task.state).ok(),
            Some(v1::DevelopmentTaskState::Failed | v1::DevelopmentTaskState::DeadlineExceeded)
        ) || task.attempt_count >= self.max_attempts
        {
            return Err(CleanStoreError::TaskNotActionable);
        }
        let now_timestamp = unix_timestamp()?;
        let now = now_timestamp.seconds;
        if task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.deadline_at.as_ref())
            .is_some_and(|deadline| timestamp_at_or_before(deadline, &now_timestamp))
        {
            return Err(CleanStoreError::DeadlineExpired);
        }
        let attempt_number = task.attempt_count.saturating_add(1);
        let attempt_id = resource_id("attempt");
        let attempt = v1::DevelopmentAttempt {
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
                params![key, digest, request.encode_to_vec(), attempt_id, now],
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
        Ok(attempt)
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
            let due = task
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.deadline_at.as_ref())
                .is_some_and(|deadline| timestamp_at_or_before(deadline, now));
            if !due {
                continue;
            }
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
            let mut attempt = task
                .active_attempt_id
                .as_deref()
                .map(|id| self.development_attempt(id))
                .transpose()?;
            if let Some(value) = attempt.as_mut()
                && !is_terminal_attempt(value.state)
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
            if let Some(value) = attempt {
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
