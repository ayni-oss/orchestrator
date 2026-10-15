use super::{
    ExecutionBackend, ExecutionRequest, RuntimeExpectation, RuntimeReconciler, now, operation_name,
    v1,
};
use ayni_orchestrator_application::{
    reconciliation::{CodingAction, can_resume_worker, coding_result_action},
    tasks::lifecycle::AttemptState,
};
impl<E: ExecutionBackend> RuntimeReconciler<E> {
    async fn validate_coding_profile_binding(
        &self,
        profile: Option<&v1::FrozenAgentProfile>,
    ) -> Result<Option<ayni_orchestrator_runtime::agent_profile::FrozenProfile>, &'static str> {
        if let Some(profile) = profile {
            let profile = profile.clone();
            self.with_database(move |store| {
                store
                    .validate_profile_connection(&profile)
                    .map_err(|_| "coding_model_binding_revoked")
            })
            .await?;
        }
        profile
            .map(crate::agent_profiles::frozen)
            .transpose()
            .map_err(|_| "coding_profile_invalid")
    }
    pub(super) async fn coding(&self, profile_id: &str) -> Result<(), &'static str> {
        let lookup_profile = profile_id.to_owned();
        let attempts = self
            .with_database(move |store| {
                store
                    .coding_attempts(&lookup_profile)
                    .map_err(|_| "coding_state_invalid")
            })
            .await?;
        for attempt in attempts.into_iter().take(self.configuration.concurrency) {
            self.reconcile_coding_attempt(attempt, profile_id).await?;
        }
        Ok(())
    }

    pub(super) async fn reconcile_coding_attempt(
        &self,
        attempt: v1::DevelopmentAttempt,
        profile_id: &str,
    ) -> Result<(), &'static str> {
        let name = operation_name(&attempt.id, 0);
        // Read output after observing Job status, so terminal records are collected
        // before any cleanup. The same path also collects live progress.
        // Publication retry must stop the prior writer before changing its generation.
        if attempt.state == v1::DevelopmentAttemptState::Queued as i32
            && attempt.publication_only
            && attempt.result.is_some()
        {
            self.kubernetes.cleanup(&name).await?;
            if self.kubernetes.coding_stopped(&name).await? {
                let id = attempt.id.clone();
                self.with_database(move |s| {
                    s.stop_coding_execution(&id)
                        .map_err(|_| "coding_state_invalid")
                })
                .await?;
                self.start_coding_attempt(&attempt, profile_id, &name)
                    .await?;
            }
            return Ok(());
        }
        let lookup = attempt.id.clone();
        let admission = self
            .with_database(move |s| {
                s.coding_admission(&lookup)
                    .map_err(|_| "coding_authority_invalid")
            })
            .await?;
        let (attempt, interrupted) = if admission.is_some() {
            let (terminal, interrupted) = self.consume_coding_reports(&attempt, &name).await?;
            let id = attempt.id.clone();
            let fresh = self
                .with_database(move |s| {
                    s.development_attempt(&id)
                        .map_err(|_| "coding_state_invalid")
                })
                .await?;
            if terminal
                || ![
                    v1::DevelopmentAttemptState::Queued as i32,
                    v1::DevelopmentAttemptState::Running as i32,
                ]
                .contains(&fresh.state)
            {
                self.cleanup_coding_attempt(&fresh, &name).await?;
                return Ok(());
            }
            (fresh, interrupted)
        } else {
            let attempt = self.collect_activity(attempt, &name).await?;
            (
                self.collect_progress(attempt, &name).await?,
                worker_interrupted(&name).await?,
            )
        };
        let job_state = self.kubernetes.coding_state(&name).await?;
        let current = attempt_state(attempt.state)?;

        let action = coding_result_action(current, job_state, interrupted);
        if action == CodingAction::Cleanup {
            self.cleanup_coding_attempt(&attempt, &name).await?;
            return Ok(());
        }
        let snapshot = attempt.snapshot.as_ref().ok_or("coding_snapshot_missing")?;
        let frozen = snapshot.runtime.as_ref().ok_or("coding_snapshot_missing")?;
        match profile_kind(frozen.profile_kind)? {
            ayni_orchestrator_application::tasks::ProfileKind::AyniManaged => {}
            ayni_orchestrator_application::tasks::ProfileKind::CustomRuntime => return Ok(()),
        }
        self.apply_coding_action(action, attempt, profile_id, &name)
            .await
    }

    async fn apply_coding_action(
        &self,
        action: CodingAction,
        attempt: v1::DevelopmentAttempt,
        profile_id: &str,
        name: &str,
    ) -> Result<(), &'static str> {
        match action {
            CodingAction::LostExecution => {
                if !self
                    .resume_retained_worker(&attempt, profile_id, name)
                    .await?
                {
                    self.coding_failed(&attempt, "coding_job_lost").await?;
                }
            }
            CodingAction::Start => {
                self.start_coding_attempt(&attempt, profile_id, name)
                    .await?;
            }
            CodingAction::WorkerFailed => {
                self.reconcile_failed_worker(&attempt, profile_id, name)
                    .await?;
            }
            CodingAction::WorkerInterrupted => {
                self.reconcile_interrupted_worker(&attempt, profile_id, name)
                    .await?;
            }
            CodingAction::CollectAndCleanup => {
                self.finish_coding_attempt(&attempt, name).await?;
                let fresh = self
                    .with_database(move |store| {
                        store
                            .development_attempt(&attempt.id)
                            .map_err(|_| "coding_state_invalid")
                    })
                    .await?;
                self.cleanup_coding_attempt(&fresh, name).await?;
            }
            CodingAction::ConfirmRetainedResult => {
                self.confirm_retained_result(&attempt.id).await?;
            }
            CodingAction::Wait => {
                self.refresh_attempt_credentials(
                    attempt.snapshot.as_ref().ok_or("coding_snapshot_missing")?,
                    name,
                )
                .await?;
            }
            CodingAction::Cleanup => {}
        }
        Ok(())
    }

    async fn confirm_retained_result(&self, id: &str) -> Result<(), &'static str> {
        let id = id.to_owned();
        self.with_database(move |store| {
            if !store
                .coding_result_exists(&id)
                .map_err(|_| "coding_state_invalid")?
            {
                return Err("coding_completed_job_missing");
            }
            store
                .coding_cleanup_complete(&id)
                .map_err(|_| "coding_state_invalid")
        })
        .await
    }

    async fn reconcile_interrupted_worker(
        &self,
        attempt: &v1::DevelopmentAttempt,
        profile_id: &str,
        name: &str,
    ) -> Result<(), &'static str> {
        if self
            .resume_retained_worker(attempt, profile_id, name)
            .await?
        {
            return Ok(());
        }
        if attempt
            .snapshot
            .as_ref()
            .is_some_and(|s| coding_seconds_remaining(s) == 0)
        {
            let id = attempt.id.clone();
            let version = attempt.resource_version;
            self.with_database(move |store| {
                store
                    .transition_attempt(
                        &id,
                        version,
                        v1::DevelopmentAttemptState::DeadlineExceeded,
                        None,
                    )
                    .map_err(|_| "coding_state_changed")
            })
            .await?;
        } else {
            self.coding_failed(
                attempt,
                "Worker interrupted before a recoverable generation was retained.",
            )
            .await?;
        }
        Ok(())
    }

    async fn reconcile_failed_worker(
        &self,
        attempt: &v1::DevelopmentAttempt,
        profile_id: &str,
        name: &str,
    ) -> Result<(), &'static str> {
        let finish = self.finish_coding_attempt(attempt, name).await;
        if finish == Err("coding_report_pending") {
            return Err("coding_report_pending");
        }
        if finish.is_ok() {
            let id = attempt.id.clone();
            let fresh = self
                .with_database(move |store| {
                    store
                        .development_attempt(&id)
                        .map_err(|_| "coding_state_invalid")
                })
                .await?;
            self.cleanup_coding_attempt(&fresh, name).await?;
            return Ok(());
        }
        if self
            .resume_retained_worker(attempt, profile_id, name)
            .await?
        {
            return Ok(());
        }
        let reason = attempt.agent_activity.last().map_or_else(
                    || "Coding worker failed before activity was retained. Inspect init containers, scheduling, and image pulls.".to_owned(),
                    |event| format!("Coding worker failed. Last activity: {}.", event.message),
                );
        self.coding_failed(attempt, &reason).await?;

        Ok(())
    }

    async fn refresh_attempt_credentials(
        &self,
        snapshot: &v1::DevelopmentSnapshot,
        name: &str,
    ) -> Result<(), &'static str> {
        let time = now()?;
        if self
            .credential_refresh
            .lock()
            .await
            .get(name)
            .is_some_and(|next| *next > time)
        {
            return Ok(());
        }
        let repository_id = snapshot
            .repository
            .as_ref()
            .ok_or("coding_snapshot_missing")?
            .repository_connection_id
            .clone();
        let repository = self
            .with_database(move |store| {
                store
                    .repository_connection(&repository_id)
                    .map_err(|_| "repository_missing")
            })
            .await?;
        if let Some(v1::repository_provider::Provider::Github(github)) =
            repository.provider.and_then(|p| p.provider)
        {
            let token = self
                .source
                .publication_token(github.installation_id, &github.repository_id)
                .await?;
            self.kubernetes.refresh_credentials(name, &token).await?;
            self.credential_refresh
                .lock()
                .await
                .insert(name.to_owned(), time + 1800);
        }
        Ok(())
    }

    async fn collect_progress(
        &self,
        attempt: v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<v1::DevelopmentAttempt, &'static str> {
        let directory = std::path::Path::new("/runtime-outputs").join(name);
        let progress = tokio::task::spawn_blocking(move || {
            ayni_orchestrator_runtime::runtime_execution::read_bounded(
                &directory.join("progress.json"),
                65536,
            )
            .ok()
            .and_then(|bytes| {
                serde_json::from_slice::<ayni_orchestrator_runtime::coding::WorkerProgress>(&bytes)
                    .ok()
            })
        })
        .await
        .map_err(|_| "worker_progress_unavailable")?;
        if let Some(progress) = progress {
            let id = attempt.id.clone();
            self.with_database(move |store| {
                store
                    .collect_worker(&id, Some(super::worker_mapping::progress(progress)), None)
                    .map_err(|_| "worker_progress_invalid")
            })
            .await
        } else {
            Ok(attempt)
        }
    }

    pub(super) async fn collect_activity(
        &self,
        attempt: v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<v1::DevelopmentAttempt, &'static str> {
        use ayni_orchestrator_runtime::{
            activity::{Activity, valid},
            runtime_execution::read_bounded,
        };
        let path = std::path::Path::new("/runtime-outputs")
            .join(name)
            .join("activity.json");
        let events = tokio::task::spawn_blocking(move || {
            read_bounded(&path, 65536)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Vec<Activity>>(&bytes).ok())
                .filter(|events| valid(events))
        })
        .await
        .map_err(|_| "coding_output_unavailable")?;
        let Some(events) = events else {
            return Ok(attempt);
        };
        let activity = events
            .into_iter()
            .map(|event| v1::AgentActivity {
                sequence: event.sequence,
                observed_at: Some(prost_types::Timestamp {
                    seconds: event.timestamp_ms / 1000,
                    nanos: i32::try_from((event.timestamp_ms % 1000) * 1_000_000).unwrap_or(0),
                }),
                message: event.failure.map_or_else(
                    || event.stage.message().to_owned(),
                    |failure| format!("{}: {}", event.stage.message(), failure.message()),
                ),
                failed: event.failed,
            })
            .collect();
        self.with_database(move |store| {
            store
                .coding_activity(&attempt.id, activity)
                .map_err(|_| "coding_state_invalid")
        })
        .await
    }

    pub(super) async fn cleanup_coding_attempt(
        &self,
        attempt: &v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<(), &'static str> {
        use std::path::Path;
        let attempt = attempt.clone();
        // Preserve whatever accounting was durably emitted before a
        // cancellation or failure; absent counters remain unavailable.
        let details_id = attempt.id.clone();
        let requested_input = attempt.requested_input.clone();
        let directory = Path::new("/runtime-outputs").join(name);
        let usage = tokio::task::spawn_blocking(move || collect_coding_usage(&directory))
            .await
            .map_err(|_| "coding_output_unavailable")?;
        if let Ok(usage) = usage {
            self.with_database(move |store| {
                store
                    .coding_details(&details_id, requested_input, usage)
                    .map_err(|_| "coding_state_invalid")
            })
            .await?;
        }
        self.kubernetes.cleanup(name).await?;
        self.credential_refresh.lock().await.remove(name);
        if !self.kubernetes.coding_stopped(name).await? {
            return Ok(());
        }
        self.with_database(move |store| {
            store
                .coding_cleanup_complete(&attempt.id)
                .map_err(|_| "coding_state_invalid")
        })
        .await?;

        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "frozen worker specification construction is kept together for review"
    )]
    pub(super) async fn start_coding_attempt(
        &self,
        attempt: &v1::DevelopmentAttempt,
        profile_id: &str,
        name: &str,
    ) -> Result<(), &'static str> {
        use ayni_orchestrator_profile_ayni::coding::WorkerSpec;
        let snapshot = attempt.snapshot.as_ref().ok_or("coding_snapshot_missing")?;
        let agent_profile = self
            .validate_coding_profile_binding(snapshot.agent_profile.as_ref())
            .await?;
        let source = snapshot
            .repository
            .as_ref()
            .ok_or("coding_snapshot_missing")?;
        let frozen = snapshot.runtime.as_ref().ok_or("coding_snapshot_missing")?;
        if !frozen.registry_connection_id.is_empty() {
            self.registry_ready(&frozen.registry_connection_id).await?;
        }
        let runtime = self.configured_profile(profile_id).await?;
        let repository_id = source.repository_connection_id.clone();
        let revision_id = frozen.runtime_profile_revision_id.clone();
        let (repository, revision) = self
            .with_database(move |store| {
                Ok((
                    store
                        .repository_connection(&repository_id)
                        .map_err(|_| "repository_missing")?,
                    store
                        .runtime_profile_revision(&revision_id)
                        .map_err(|_| "runtime_revision_missing")?,
                ))
            })
            .await?;
        let Some(v1::repository_provider::Provider::Github(github)) =
            repository.provider.and_then(|p| p.provider)
        else {
            return Err("repository_provider_unsupported");
        };
        let Some(v1::runtime_profile_revision::Configuration::AyniManaged(managed)) =
            revision.configuration
        else {
            return Err("coding_snapshot_invalid");
        };
        let agent = snapshot.agent.as_ref().ok_or("coding_agent_missing")?;
        if agent.provider != "openai" {
            self.coding_failed(attempt, "coding_provider_unsupported")
                .await?;
            return Ok(());
        }
        let platform = frozen.platform.as_ref().ok_or("coding_platform_missing")?;
        let platform = format!("{}/{}", platform.operating_system, platform.architecture);
        let mut runtime = runtime.clone();
        if !frozen.registry_connection_id.is_empty() {
            runtime.pull_secret =
                (!frozen.pull_secret_name.is_empty()).then_some(frozen.pull_secret_name.clone());
        }
        runtime.platform.clone_from(&platform);
        let constraints = snapshot
            .change_constraints
            .as_ref()
            .ok_or("coding_constraints_missing")?;
        let mut request = WorkerSpec {
            profile: agent_profile,
            execution: ExecutionRequest {
                repository_url: format!("https://github.com/{}/{}.git", github.owner, github.name),
                revision: source.revision.clone(),
                expected: RuntimeExpectation {
                    lock_fingerprint: format!("sha256:{}", managed.lock_fingerprint),
                    platform,
                    ayni_version: managed.ayni_version,
                },
            },
            request: snapshot.request.clone(),
            acceptance_criteria: snapshot.acceptance_criteria.clone(),
            model: agent.model.clone(),
            provider_credentials: snapshot.provider_credentials.as_ref().map(|binding| {
                ayni_orchestrator_runtime::coding::ProviderCredentials {
                    connection_id: binding.connection_id.clone(),
                    credential_revision: binding.credential_revision.clone(),
                    secret_name: binding.secret_name.clone(),
                    secret_uid: binding.secret_uid.clone(),
                }
            }),
            task_id: attempt.development_task_id.clone(),
            attempt_id: attempt.id.clone(),
            runtime_digest: frozen.resolved_digest.clone(),
            limits: ayni_orchestrator_runtime::coding::WorkerLimits {
                coding_turns: u16::try_from(
                    snapshot
                        .limits
                        .as_ref()
                        .ok_or("worker_limits_missing")?
                        .coding_turns,
                )
                .map_err(|_| "worker_limits_invalid")?,
                quality_repair_limit: u8::try_from(
                    snapshot
                        .limits
                        .as_ref()
                        .ok_or("worker_limits_missing")?
                        .quality_repair_limit,
                )
                .map_err(|_| "worker_limits_invalid")?,
                deadline_at: snapshot
                    .limits
                    .as_ref()
                    .and_then(|l| l.deadline_at.as_ref())
                    .ok_or("worker_deadline_missing")?
                    .seconds,
            },
            publication: ayni_orchestrator_runtime::coding::PublicationDestination {
                repository: format!("{}/{}", github.owner, github.name),
                base_branch: snapshot.publication_base_branch.clone(),
                branch: format!("task/{}", attempt.id.replace(':', "-")),
            },
            publication_only: attempt.publication_only,
            reporting: None,
            constraints: ayni_orchestrator_domain::ChangeConstraints {
                protected_paths: constraints.protected_paths.clone(),
                allow_test_edits: constraints.allow_test_edits,
            },
        };
        // Select authority before Kubernetes creation; crash replay adopts the same generation.
        let id = attempt.id.clone();
        let execution = name.to_owned();
        let configuration = (self.configuration.verification_mode
            == crate::process::runtime_configuration::VerificationMode::Events)
            .then(|| self.configuration.reporting.clone())
            .flatten();
        request.reporting = self
            .with_database(move |s| {
                s.select_coding_execution(&id, &execution, configuration)
                    .map_err(|_| "coding_authority_invalid")
            })
            .await?;
        if let Some(admission) = &request.reporting {
            let digest = ayni_orchestrator_domain::digest::sha256(
                &serde_json::to_vec(&request).map_err(|_| "coding_request_invalid")?,
            );
            let generation = admission.binding.generation.clone();
            let id = attempt.id.clone();
            self.with_database(move |s| {
                s.bind_coding_request(&id, &generation, &digest)
                    .map_err(|_| "coding_authority_invalid")
            })
            .await?;
        }
        let remaining = coding_seconds_remaining(snapshot);
        if remaining == 0 {
            return Ok(());
        }
        let token = self
            .source
            .publication_token(github.installation_id, &github.repository_id)
            .await?;
        // The existing immutable attempt precedes creation. Repeat creation uses
        // the same name and exact request, including after an uncertain response.
        self.kubernetes
            .start_coding(
                name,
                &runtime.execution_runtime(),
                &request,
                &token,
                &format!("{}@{}", frozen.oci_repository, frozen.resolved_digest),
                u32::try_from(remaining.min(86400)).map_err(|_| "coding_deadline_invalid")?,
            )
            .await?;
        if attempt.state == v1::DevelopmentAttemptState::Running as i32 {
            return Ok(());
        }
        let attempt_id = attempt.id.clone();
        let attempt_version = attempt.resource_version;
        self.with_database(move |store| {
            store
                .transition_attempt(
                    &attempt_id,
                    attempt_version,
                    v1::DevelopmentAttemptState::Running,
                    None,
                )
                .map_err(|_| "coding_state_changed")
        })
        .await?;
        Ok(())
    }

    pub(super) async fn finish_coding_attempt(
        &self,
        attempt: &v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<(), &'static str> {
        let id = attempt.id.clone();
        if self
            .with_database(move |s| {
                s.coding_admission(&id)
                    .map_err(|_| "coding_authority_invalid")
            })
            .await?
            .is_some()
        {
            self.recover_coding_receipt(name).await?;
            let (terminal, _) = self.consume_coding_reports(attempt, name).await?;
            return if terminal {
                Ok(())
            } else {
                Err("coding_report_pending")
            };
        }
        let directory = std::path::Path::new("/runtime-outputs").join(name);
        let (result, usage) = tokio::task::spawn_blocking(move || {
            let result = collect_worker_result(&directory)?;
            Ok::<_, &'static str>((result, collect_coding_usage(&directory)?))
        })
        .await
        .map_err(|_| "coding_output_unavailable")??;
        let id = attempt.id.clone();
        self.with_database(move |store| {
            store
                .collect_worker(
                    &id,
                    None,
                    Some(super::worker_mapping::result(result, usage)),
                )
                .map_err(|_| "worker_result_rejected")
        })
        .await?;
        Ok(())
    }

    async fn resume_retained_worker(
        &self,
        attempt: &v1::DevelopmentAttempt,
        profile_id: &str,
        name: &str,
    ) -> Result<bool, &'static str> {
        let directory = std::path::Path::new("/runtime-outputs").join(name);
        let retained = tokio::task::spawn_blocking(move || {
            let pointer = ayni_orchestrator_runtime::runtime_execution::read_bounded(
                &directory.join("recovery-current"),
                128,
            )
            .ok()?;
            let generation = std::str::from_utf8(&pointer).ok()?;
            if !generation.starts_with("generation-")
                || !generation
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return None;
            }
            let state = directory
                .join("recovery")
                .join(generation)
                .join("state.json");
            ayni_orchestrator_runtime::runtime_execution::read_bounded(&state, 131_072).ok()?;
            Some(())
        })
        .await
        .map_err(|_| "worker_recovery_unavailable")?
        .is_some();
        let before_deadline = attempt
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| coding_seconds_remaining(snapshot) > 0);
        if !can_resume_worker(attempt_state(attempt.state)?, retained, before_deadline) {
            return Ok(false);
        }
        // Recreate execution resources only after the prior pod has stopped. The
        // retained attempt output and immutable specification preserve budgets.
        self.kubernetes.cleanup(name).await?;
        if !self.kubernetes.coding_stopped(name).await? {
            return Ok(true);
        }
        let id = attempt.id.clone();
        self.with_database(move |s| {
            s.stop_coding_execution(&id)
                .map_err(|_| "coding_state_invalid")
        })
        .await?;
        self.start_coding_attempt(attempt, profile_id, name).await?;
        Ok(true)
    }

    pub(super) async fn coding_failed(
        &self,
        attempt: &v1::DevelopmentAttempt,
        reason: &str,
    ) -> Result<(), &'static str> {
        let id = attempt.id.clone();
        let version = attempt.resource_version;
        let reason = reason.to_owned();
        self.with_database(move |store| {
            store
                .transition_attempt(
                    &id,
                    version,
                    v1::DevelopmentAttemptState::Failed,
                    Some(v1::ResourceFailure {
                        reason: v1::ErrorReason::RuntimeArtifactNotVerified as i32,
                        message: reason,
                        remediation: Some(
                            "Inspect the isolated Job, repair the cause, then retry the task."
                                .into(),
                        ),
                        observed_at: None,
                    }),
                )
                .map_err(|_| "coding_state_changed")
        })
        .await?;
        Ok(())
    }
}

pub(super) fn collect_worker_result(
    directory: &std::path::Path,
) -> Result<ayni_orchestrator_runtime::coding::WorkerResult, &'static str> {
    use ayni_orchestrator_runtime::coding::WorkerResult;
    let bytes = ayni_orchestrator_runtime::runtime_execution::read_bounded(
        &directory.join("result.json"),
        262_144,
    )
    .map_err(|_| "worker_result_unavailable")?;
    let result: WorkerResult =
        serde_json::from_slice(&bytes).map_err(|_| "worker_result_invalid")?;
    validate_worker_evidence(directory, &result)?;
    Ok(result)
}
pub(super) fn validate_worker_evidence(
    directory: &std::path::Path,
    result: &ayni_orchestrator_runtime::coding::WorkerResult,
) -> Result<(), &'static str> {
    use ayni_orchestrator_runtime::coding::QualityOutcome;
    use sha2::{Digest, Sha256};
    if let Some(ref evidence) = result.quality.evidence {
        if evidence.artifact_path != "evidence/signals.json"
            || Some(&evidence.commit) != result.commit.as_ref()
        {
            return Err("worker_evidence_invalid");
        }
        let artifact = ayni_orchestrator_runtime::runtime_execution::read_bounded(
            &directory.join("evidence/signals.json"),
            4 * 1024 * 1024,
        )
        .map_err(|_| "worker_evidence_missing")?;
        let artifact_value: serde_json::Value =
            serde_json::from_slice(&artifact).map_err(|_| "worker_evidence_invalid")?;
        if evidence.signals_digest != format!("sha256:{:x}", Sha256::digest(&artifact))
            || artifact_value["source_fingerprint"].as_str()
                != Some(evidence.source_fingerprint.as_str())
            || artifact_value["invocation"]["command"] != "check"
            || artifact_value["completion"]["scope"] != "repository"
        {
            return Err("worker_evidence_invalid");
        }
    } else if result.quality.outcome == QualityOutcome::Passed {
        return Err("worker_evidence_missing");
    }
    Ok(())
}

pub(super) fn collect_coding_usage(
    directory: &std::path::Path,
) -> Result<v1::AttemptModelUsage, &'static str> {
    use ayni_orchestrator_domain::ModelUsageReport;
    let mut reports = Vec::new();
    if std::fs::symlink_metadata(directory.join("usage")).is_ok_and(|meta| !meta.is_dir()) {
        return Err("coding_usage_invalid");
    }
    for number in 1..=64 {
        let path = directory.join(format!("usage/coding-call-{number}.json"));
        if !path.exists() {
            break;
        }
        let report: ModelUsageReport = serde_json::from_slice(
            &ayni_orchestrator_runtime::runtime_execution::read_bounded(&path, 16384)
                .map_err(|_| "coding_usage_invalid")?,
        )
        .map_err(|_| "coding_usage_invalid")?;
        reports.push(report);
    }
    coding_usage_reports(reports)
}
pub(super) fn coding_usage_reports(
    reports: Vec<ayni_orchestrator_domain::ModelUsageReport>,
) -> Result<v1::AttemptModelUsage, &'static str> {
    use ayni_orchestrator_domain::ModelInteractionOutcome;
    if reports.len() > 64 {
        return Err("coding_usage_invalid");
    }
    let mut interactions = Vec::new();
    for report in reports {
        interactions.push(v1::ModelUsageInteraction {
            call_id: report.call_id,
            stage: v1::ModelUsageStage::Coding as i32,
            provider: report.provider,
            model: report.model,
            resolved_model: report.resolved_model,
            started_at: Some(milliseconds(report.started_at_ms)),
            completed_at: report.completed_at_ms.map(milliseconds),
            latency: report.latency_ms.map(|ms| prost_types::Duration {
                seconds: i64::try_from(ms / 1000).unwrap_or(i64::MAX),
                nanos: i32::try_from((ms % 1000) * 1_000_000).unwrap_or(0),
            }),
            outcome: match report.outcome {
                ModelInteractionOutcome::Succeeded => v1::ModelInteractionOutcome::Succeeded,
                ModelInteractionOutcome::Cancelled => v1::ModelInteractionOutcome::Cancelled,
                _ => v1::ModelInteractionOutcome::Failed,
            } as i32,
            tokens: Some(v1::ProviderTokenUsage {
                input_tokens: report.tokens.input_tokens,
                output_tokens: report.tokens.output_tokens,
                cached_input_tokens: report.tokens.cached_input_tokens,
                reasoning_tokens: report.tokens.reasoning_tokens,
                total_tokens: report.tokens.total_tokens,
            }),
            estimated_cost: None,
        });
    }
    Ok(v1::AttemptModelUsage {
        collection_status: if interactions.is_empty() {
            v1::ModelUsageCollectionStatus::Unavailable
        } else {
            v1::ModelUsageCollectionStatus::Complete
        } as i32,
        interactions,
        by_stage: vec![],
        total: None,
    })
}
pub(super) fn milliseconds(ms: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: ms.div_euclid(1000),
        nanos: i32::try_from(ms.rem_euclid(1000) * 1_000_000).unwrap_or(0),
    }
}

pub(super) fn coding_seconds_remaining(snapshot: &v1::DevelopmentSnapshot) -> u64 {
    snapshot.deadline_at.as_ref().map_or(3600, |t| {
        u64::try_from(t.seconds)
            .unwrap_or(0)
            .saturating_sub(now().unwrap_or(u64::MAX))
    })
}

fn attempt_state(value: i32) -> Result<AttemptState, &'static str> {
    use v1::DevelopmentAttemptState as Wire;
    Ok(
        match Wire::try_from(value).map_err(|_| "coding_state_invalid")? {
            Wire::Queued => AttemptState::Queued,
            Wire::Running => AttemptState::Running,
            Wire::WaitingForInput => AttemptState::WaitingForInput,
            Wire::Succeeded => AttemptState::Succeeded,
            Wire::Failed => AttemptState::Failed,
            Wire::Cancelled => AttemptState::Cancelled,
            Wire::DeadlineExceeded => AttemptState::DeadlineExceeded,
            Wire::Unsupported => AttemptState::Unsupported,
            Wire::Unspecified => return Err("coding_state_invalid"),
        },
    )
}

fn profile_kind(
    value: i32,
) -> Result<ayni_orchestrator_application::tasks::ProfileKind, &'static str> {
    use ayni_orchestrator_application::tasks::ProfileKind;
    match v1::RuntimeProfileKind::try_from(value).map_err(|_| "coding_snapshot_invalid")? {
        v1::RuntimeProfileKind::AyniManaged => Ok(ProfileKind::AyniManaged),
        v1::RuntimeProfileKind::CustomRuntime => Ok(ProfileKind::CustomRuntime),
        v1::RuntimeProfileKind::Unspecified => Err("coding_snapshot_invalid"),
    }
}

async fn worker_interrupted(name: &str) -> Result<bool, &'static str> {
    let directory = std::path::Path::new("/runtime-outputs").join(name);
    tokio::task::spawn_blocking(move || {
        ayni_orchestrator_runtime::runtime_execution::read_bounded(
            &directory.join("result.json"),
            262_144,
        )
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<ayni_orchestrator_runtime::coding::WorkerResult>(&bytes).ok()
        })
        .is_some_and(|result| {
            result.execution == ayni_orchestrator_runtime::coding::ExecutionOutcome::Cancelled
        })
    })
    .await
    .map_err(|_| "worker_result_unavailable")
}

#[cfg(test)]
mod collection_tests {
    use super::collect_worker_result;
    use ayni_orchestrator_runtime::coding::{
        ExecutionOutcome, PublicationOutcome, QualityEvidence, QualityOutcome, QualityResult,
        WorkerResult,
    };

    #[test]
    fn refuses_missing_or_changed_final_full_check_artifact()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let result = WorkerResult {
            execution: ExecutionOutcome::Succeeded,
            commit: Some("a".repeat(40)),
            changed_paths: vec!["README.md".into()],
            quality: QualityResult {
                outcome: QualityOutcome::Passed,
                evidence: Some(QualityEvidence {
                    artifact_path: "evidence/signals.json".into(),
                    signals_digest: format!("sha256:{}", "b".repeat(64)),
                    source_fingerprint: format!("sha256:{}", "c".repeat(64)),
                    commit: "a".repeat(40),
                }),
                completed_targets: Some(1),
                skipped_targets: Some(0),
                warning_offenders: Some(0),
                failing_offenders: Some(0),
                rows: vec![],
                findings: vec![],
            },
            model_usage: serde_json::Value::Null,
            requested_input: None,
            publication: PublicationOutcome::Pending,
            pull_request: None,
            coding_turns_consumed: 1,
            quality_round: 0,
        };
        std::fs::write(
            directory.path().join("result.json"),
            serde_json::to_vec(&result)?,
        )?;
        assert!(collect_worker_result(directory.path()).is_err());
        std::fs::create_dir(directory.path().join("evidence"))?;
        std::fs::write(directory.path().join("evidence/signals.json"), b"{}")?;
        assert!(collect_worker_result(directory.path()).is_err());
        Ok(())
    }
}
