//! `SQLite` implementation of the atomic task-admission port.
use super::{
    CleanStore, CleanStoreError, DEVELOPMENT_SERVICE, Message, Timestamp, action_row, append_event,
    get_payload, map_constraint, params, resource_id, timestamp, unix_timestamp, v1,
};
use ayni_orchestrator_application::tasks::{
    AdmissionError, AdmissionFacts, AdmissionReceipt, AdmitTask, FrozenTask, Platform, ProfileKind,
    RequestIdentity, RuntimeBinding, TaskRepository, TimePoint,
};

pub(crate) fn admission_error(error: AdmissionError<CleanStoreError>) -> CleanStoreError {
    match error {
        AdmissionError::Storage(error) => error,
        AdmissionError::ParentMismatch => CleanStoreError::ParentMismatch,
        AdmissionError::ProfileNotReady => CleanStoreError::ProfileNotReady,
        AdmissionError::Clock => CleanStoreError::Clock,
    }
}

const fn to_wire_time(value: TimePoint) -> Timestamp {
    Timestamp {
        seconds: value.seconds,
        nanos: value.nanos,
    }
}

impl TaskRepository for CleanStore {
    type Error = CleanStoreError;

    fn replay(&self, identity: &RequestIdentity) -> Result<Option<AdmissionReceipt>, Self::Error> {
        let Some(existing) = action_row(&self.connection, "CreateDevelopmentTask", &identity.key)?
        else {
            return Ok(None);
        };
        if existing.request_hash != identity.digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        let task_id = existing.target_id.ok_or(CleanStoreError::CorruptState)?;
        let initial_attempt: v1::DevelopmentAttempt = get_payload(
            &self.connection,
            "SELECT payload FROM development_attempts WHERE development_task_id=?1 AND attempt_number=1",
            &task_id,
            "development_attempt",
        )?;
        self.development_task(&task_id)?;
        Ok(Some(AdmissionReceipt {
            task_id,
            attempt_id: initial_attempt.id,
        }))
    }

    fn admission_facts(&self, command: &AdmitTask) -> Result<AdmissionFacts, Self::Error> {
        let repository = self.project_task_repository(&v1::DevelopmentTaskRequest {
            project_id: command.project_id.clone(),
            repository_connection_id: command.repository_id.clone(),
            ..Default::default()
        })?;
        let snapshot =
            self.repository_snapshot_for_revision(&repository.id, &command.repository_revision)?;
        let profile = self.runtime_profile(&command.profile_id)?;
        let mut facts = AdmissionFacts {
            repository_id: repository.id,
            snapshot_id: snapshot.id,
            source_revision: snapshot.revision,
            base_branch: repository.default_branch,
            profile_repository_id: profile.repository_connection_id,
            profile_ready: profile.state == v1::RuntimeProfileState::Ready as i32,
            profile_operational: profile.support == v1::RuntimeProfileSupport::Operational as i32,
            profile_source_revision: String::new(),
            artifact_revision_id: String::new(),
            artifact_verified: false,
            runtime: None,
        };
        if facts.profile_repository_id != facts.repository_id
            || !facts.profile_ready
            || !facts.profile_operational
        {
            return Ok(facts);
        }
        let (Some(revision_id), Some(artifact_id)) = (
            profile.active_revision_id,
            profile.active_runtime_artifact_id,
        ) else {
            return Ok(facts);
        };
        let revision = self.runtime_profile_revision(&revision_id)?;
        let artifact = self.runtime_artifact(&artifact_id)?;
        facts.profile_source_revision = revision.repository_revision;
        facts.artifact_revision_id = artifact.runtime_profile_revision_id;
        facts.artifact_verified = artifact.status == v1::RuntimeArtifactStatus::Verified as i32;
        facts.runtime = Some(RuntimeBinding {
            profile_id: profile.id,
            revision_id: revision.id,
            kind: match v1::RuntimeProfileKind::try_from(profile.kind) {
                Ok(v1::RuntimeProfileKind::AyniManaged) => ProfileKind::AyniManaged,
                Ok(v1::RuntimeProfileKind::CustomRuntime) => ProfileKind::CustomRuntime,
                _ => return Err(CleanStoreError::CorruptState),
            },
            artifact_id: artifact.id,
            oci_repository: artifact.oci_repository,
            digest: artifact.resolved_digest,
            platform: artifact.platform.map(|p| Platform {
                operating_system: p.operating_system,
                architecture: p.architecture,
                variant: p.variant,
            }),
        });
        Ok(facts)
    }

    fn now(&self) -> Result<TimePoint, Self::Error> {
        let now = unix_timestamp()?;
        Ok(TimePoint {
            seconds: now.seconds,
            nanos: now.nanos,
        })
    }

    fn commit_admission(
        &mut self,
        identity: RequestIdentity,
        frozen: FrozenTask,
        created_at: TimePoint,
    ) -> Result<AdmissionReceipt, Self::Error> {
        let now = created_at.seconds;
        let repository_id = frozen.repository_id;
        let profile_id = frozen.runtime.profile_id;
        let repository = self.repository_connection(&repository_id)?;
        let registry = if repository.registry_connection_id.is_empty() {
            None
        } else {
            self.validate_registry_selection(
                &repository.registry_connection_id,
                &repository.image_namespace,
            )?;
            Some(self.get_connection(&repository.registry_connection_id)?)
        };
        if let Some(registry) = &registry {
            let Some(v1::repository_provider::Provider::Github(github)) =
                repository.provider.and_then(|p| p.provider)
            else {
                return Err(CleanStoreError::InvalidTransition);
            };
            let expected = format!(
                "ghcr.io/{}/{}/{}-ayni",
                registry.image_namespace,
                github.owner.to_lowercase(),
                github.name.to_lowercase()
            );
            if frozen.runtime.oci_repository != expected {
                return Err(CleanStoreError::InvalidTransition);
            }
        }
        let snapshot = v1::DevelopmentSnapshot {
            repository: Some(v1::FrozenRepository {
                repository_connection_id: repository_id.clone(),
                repository_snapshot_id: frozen.snapshot_id,
                revision: frozen.source_revision,
            }),
            runtime: Some(v1::FrozenRuntime {
                runtime_profile_id: profile_id.clone(),
                runtime_profile_revision_id: frozen.runtime.revision_id,
                profile_kind: match frozen.runtime.kind {
                    ProfileKind::AyniManaged => v1::RuntimeProfileKind::AyniManaged as i32,
                    ProfileKind::CustomRuntime => v1::RuntimeProfileKind::CustomRuntime as i32,
                },
                runtime_artifact_id: frozen.runtime.artifact_id,
                oci_repository: frozen.runtime.oci_repository,
                resolved_digest: frozen.runtime.digest,
                registry_connection_id: registry.as_ref().map(|v| v.id.clone()).unwrap_or_default(),
                registry_revision: registry
                    .as_ref()
                    .map(|v| v.observed_revision.clone())
                    .unwrap_or_default(),
                pull_secret_name: registry
                    .as_ref()
                    .map(|v| v.pull_secret_name.clone())
                    .unwrap_or_default(),
                platform: frozen.runtime.platform.map(|p| v1::OciPlatform {
                    operating_system: p.operating_system,
                    architecture: p.architecture,
                    variant: p.variant,
                }),
            }),
            agent: frozen.agent.map(|a| v1::AgentSelection {
                provider: a.provider,
                agent: a.agent,
                model: a.model,
            }),
            request: frozen.request,
            acceptance_criteria: frozen.acceptance_criteria,
            change_constraints: frozen.constraints.map(|c| v1::ChangeConstraints {
                protected_paths: c.protected_paths,
                allow_test_edits: c.allow_test_edits,
            }),
            deadline_at: frozen
                .deadline_at
                .map(to_wire_time)
                .or_else(|| Some(timestamp(now + 3600))),
            limits: Some(v1::WorkerLimits {
                coding_turns: self.coding_turns,
                quality_repair_limit: self.quality_repair_limit,
                deadline_at: frozen
                    .deadline_at
                    .map(to_wire_time)
                    .or_else(|| Some(timestamp(now + 3600))),
            }),
            publication_base_branch: frozen.base_branch,
        };
        let task_id = resource_id("task");
        let attempt_id = resource_id("attempt");
        let task = v1::DevelopmentTask {
            project_id: frozen.project_id,
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
            agent_activity: vec![],
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
            progress: None,
            result: None,
            pull_request: None,
            publication_only: false,
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
                params![identity.key, identity.digest, identity.payload, task_id, now],
            )
            .map_err(map_constraint)?;
        let task_cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_task",
            &task_id,
            1,
            Some(&repository_id),
            Some(&task_id),
            &task,
        )?;
        transaction
            .execute(
                "INSERT INTO development_tasks(
                    id,repository_connection_id,runtime_profile_id,state,active_attempt_id,
                    deadline_at_seconds,deadline_at_nanos,payload,resource_version,event_cursor,project_id
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9,?10)",
                params![
                    task_id,
                    repository_id,
                    profile_id,
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
                    i64::try_from(task_cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    task.project_id
                ],
            )
            .map_err(map_constraint)?;
        let attempt_cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "development_attempt",
            &attempt_id,
            1,
            Some(&repository_id),
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
        Ok(AdmissionReceipt {
            task_id,
            attempt_id,
        })
    }
}
