impl RuntimeReconciler {
    async fn coding(&self, profile_id: &str) -> Result<(), &'static str> {
        let attempts = self
            .state
            .store
            .lock()
            .map_err(|_| "runtime_store_unavailable")?
            .coding_attempts(profile_id)
            .map_err(|_| "coding_state_invalid")?;
        for attempt in attempts.into_iter().take(self.configuration.concurrency) {
            self.reconcile_coding_attempt(attempt, profile_id).await?;
        }
        Ok(())
    }

    async fn reconcile_coding_attempt(&self, attempt: v1::DevelopmentAttempt, profile_id: &str) -> Result<(), &'static str> {
        let name = operation_name(&attempt.id, 0);
        // Read output after observing Job status, so terminal records are collected
        // before any cleanup. The same path also collects live progress.
        let job_state = self.kubernetes.coding_state(&name).await?;
        let attempt = self.collect_activity(attempt, &name)?;
        let current = v1::DevelopmentAttemptState::try_from(attempt.state)
            .map_err(|_| "coding_state_invalid")?;
        if coding_needs_cleanup(current) {
            self.cleanup_coding_attempt(&attempt, &name).await?;
            return Ok(());
        }
        let snapshot = attempt.snapshot.as_ref().ok_or("coding_snapshot_missing")?;
        let frozen = snapshot.runtime.as_ref().ok_or("coding_snapshot_missing")?;
        if frozen.profile_kind != v1::RuntimeProfileKind::AyniManaged as i32 {
            return Ok(());
        }
        match job_state {
            "missing" if current == v1::DevelopmentAttemptState::Running => {
                self.coding_failed(&attempt, "coding_job_lost")?;
            }
            "missing" if current == v1::DevelopmentAttemptState::Queued => {
                self.start_coding_attempt(&attempt, profile_id, &name)
                    .await?;
            }
            "failed" => {
                let reason = attempt.agent_activity.last().map_or_else(
                    || "Coding worker failed before activity was retained. Inspect init containers, scheduling, and image pulls.".to_owned(),
                    |event| format!("Coding worker failed. Last activity: {}.", event.message),
                );
                self.coding_failed(&attempt, &reason)?;
            }
            "succeeded" => {
                self.finish_coding_attempt(&attempt, &name)?;
                let fresh = self.state.store.lock().map_err(|_| "runtime_store_unavailable")?
                    .development_attempt(&attempt.id).map_err(|_| "coding_state_invalid")?;
                self.cleanup_coding_attempt(&fresh, &name).await?;
            }
            "missing" => {
                let mut store = self
                    .state
                    .store
                    .lock()
                    .map_err(|_| "runtime_store_unavailable")?;
                if store
                    .coding_candidate_exists(&attempt.id)
                    .map_err(|_| "coding_state_invalid")?
                {
                    store
                        .coding_cleanup_complete(&attempt.id)
                        .map_err(|_| "coding_state_invalid")?;
                } else {
                    return Err("coding_completed_job_missing");
                }
                drop(store);
            }
            _ => {}
        }
        Ok(())
    }


    fn collect_activity(&self, attempt: v1::DevelopmentAttempt, name: &str) -> Result<v1::DevelopmentAttempt, &'static str> {
        use ayni_orchestrator_runtime::{activity::{Activity, valid}, runtime_execution::read_bounded};
        let path = std::path::Path::new("/runtime-outputs").join(name).join("activity.json");
        let events = read_bounded(&path, 65536).ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Activity>>(&bytes).ok())
            .filter(|events| valid(events));
        let Some(events) = events else { return Ok(attempt); };
        let activity = events.into_iter().map(|event| v1::AgentActivity {
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
        }).collect();
        self.state.store.lock().map_err(|_| "runtime_store_unavailable")?
            .coding_activity(&attempt.id, activity).map_err(|_| "coding_state_invalid")
    }

    async fn cleanup_coding_attempt(
        &self,
        attempt: &v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<(), &'static str> {
        use std::path::Path;
        // Preserve whatever accounting was durably emitted before a
        // cancellation or failure; absent counters remain unavailable.
        if let Ok(usage) = collect_coding_usage(&Path::new("/runtime-outputs").join(name)) {
            self.state
                .store
                .lock()
                .map_err(|_| "runtime_store_unavailable")?
                .coding_details(&attempt.id, attempt.requested_input.clone(), usage)
                .map_err(|_| "coding_state_invalid")?;
        }
        self.kubernetes.cleanup(name).await?;
        if !self.kubernetes.coding_stopped(name).await? { return Ok(()); }
        if [v1::DevelopmentAttemptState::Failed as i32,
            v1::DevelopmentAttemptState::DeadlineExceeded as i32,
            v1::DevelopmentAttemptState::WaitingForInput as i32].contains(&attempt.state) {
            self.recover_coding_checkpoint(attempt, name)?;
        }
        self.state
            .store
            .lock()
            .map_err(|_| "runtime_store_unavailable")?
            .coding_cleanup_complete(&attempt.id)
            .map_err(|_| "coding_state_invalid")?;

        Ok(())
    }

    fn recover_coding_checkpoint(&self, attempt: &v1::DevelopmentAttempt, name: &str) -> Result<(), &'static str> {
        use ayni_orchestrator_runtime::runtime_execution::read_bounded;
        // Once publication owns a candidate, retries must not replace its bundle.
        if self.state.store.lock().map_err(|_| "runtime_store_unavailable")?
            .coding_candidate_exists(&attempt.id).map_err(|_| "coding_state_invalid")? {
            return Ok(());
        }
        let directory = std::path::Path::new("/runtime-outputs").join(name);
        let pointer = match read_bounded(&directory.join("checkpoint"), 40) {
            Ok(pointer) => pointer,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("coding_checkpoint_unavailable"),
        };
        let sha = std::str::from_utf8(&pointer).map_err(|_| "coding_checkpoint_invalid")?;
        if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("coding_checkpoint_invalid");
        }
        let checkpoint = directory.join("checkpoints").join(sha);
        let (report, _) = collect_coding_result(&checkpoint)?;
        if report.commit.as_deref() != Some(sha) { return Err("coding_checkpoint_invalid"); }
        let source = attempt.snapshot.as_ref().and_then(|s| s.repository.as_ref()).ok_or("coding_snapshot_missing")?;
        self.state.store.lock().map_err(|_| "runtime_store_unavailable")?
            .record_candidate(v1::Candidate {
                development_attempt_id: attempt.id.clone(),
                repository_connection_id: source.repository_connection_id.clone(),
                base_revision: source.revision.clone(),
                identity: Some(v1::CandidateIdentity { identity: Some(v1::candidate_identity::Identity::Git(
                    v1::GitCandidate { commit_sha: sha.into() })) }),
                state: v1::CandidateState::Available as i32,
                changed_paths: report.changed_paths,
                coding_advisory: Some(report.advisory),
                ..Default::default()
            }).map_err(|_| "coding_candidate_rejected")?;
        Ok(())
    }

    async fn start_coding_attempt(
        &self,
        attempt: &v1::DevelopmentAttempt,
        profile_id: &str,
        name: &str,
    ) -> Result<(), &'static str> {
        use ayni_orchestrator_profile_ayni::coding::CodingRequest;
        let snapshot = attempt.snapshot.as_ref().ok_or("coding_snapshot_missing")?;
        let source = snapshot
            .repository
            .as_ref()
            .ok_or("coding_snapshot_missing")?;
        let frozen = snapshot.runtime.as_ref().ok_or("coding_snapshot_missing")?;
        let runtime = self.configured_profile(profile_id)?;
        let (repository, revision) = {
            let store = self
                .state
                .store
                .lock()
                .map_err(|_| "runtime_store_unavailable")?;
            (
                store
                    .repository_connection(&source.repository_connection_id)
                    .map_err(|_| "repository_missing")?,
                store
                    .runtime_profile_revision(&frozen.runtime_profile_revision_id)
                    .map_err(|_| "runtime_revision_missing")?,
            )
        };
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
            self.coding_failed(attempt, "coding_provider_unsupported")?;
            return Ok(());
        }
        let platform = frozen.platform.as_ref().ok_or("coding_platform_missing")?;
        let platform = format!("{}/{}", platform.operating_system, platform.architecture);
        let mut runtime = runtime.clone();
        runtime.platform.clone_from(&platform);
        let constraints = snapshot
            .change_constraints
            .as_ref()
            .ok_or("coding_constraints_missing")?;
        let request = CodingRequest {
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
            constraints: ayni_orchestrator_domain::ChangeConstraints {
                protected_paths: constraints.protected_paths.clone(),
                allow_test_edits: constraints.allow_test_edits,
            },
        };
        let remaining = coding_seconds_remaining(snapshot);
        if remaining == 0 {
            return Ok(());
        }
        let token = self
            .source
            .token(github.installation_id, &github.repository_id)
            .await?;
        // The existing immutable attempt precedes creation. Repeat creation uses
        // the same name and exact request, including after an uncertain response.
        self.kubernetes
            .start_coding(
                name,
                &runtime,
                &request,
                &token,
                &format!("{}@{}", frozen.oci_repository, frozen.resolved_digest),
                u32::try_from(remaining.min(86400)).map_err(|_| "coding_deadline_invalid")?,
            )
            .await?;
        self.state
            .store
            .lock()
            .map_err(|_| "runtime_store_unavailable")?
            .transition_attempt(
                &attempt.id,
                attempt.resource_version,
                v1::DevelopmentAttemptState::Running,
                None,
            )
            .map_err(|_| "coding_state_changed")?;
        Ok(())
    }

    fn finish_coding_attempt(
        &self,
        attempt: &v1::DevelopmentAttempt,
        name: &str,
    ) -> Result<(), &'static str> {
        use std::path::Path;
        let snapshot = attempt.snapshot.as_ref().ok_or("coding_snapshot_missing")?;
        let source = snapshot
            .repository
            .as_ref()
            .ok_or("coding_snapshot_missing")?;
        let directory = Path::new("/runtime-outputs").join(name);
        let (report, usage) = match collect_coding_result(&directory) {
            Ok(result) => result,
            Err(reason) => {
                self.coding_failed(attempt, reason)?;
                return Ok(());
            }
        };
        {
            let mut store = self
                .state
                .store
                .lock()
                .map_err(|_| "runtime_store_unavailable")?;
            let fresh = store
                .development_attempt(&attempt.id)
                .map_err(|_| "coding_state_invalid")?;
            if !matches!(
                v1::DevelopmentAttemptState::try_from(fresh.state),
                Ok(v1::DevelopmentAttemptState::Running
                    | v1::DevelopmentAttemptState::Queued
                    | v1::DevelopmentAttemptState::Succeeded)
            ) {
                return Ok(());
            }
            let next = if report.requested_input.is_some() {
                v1::DevelopmentAttemptState::WaitingForInput
            } else {
                v1::DevelopmentAttemptState::Succeeded
            };
            // Creation may have completed before the Running transition was saved.
            let fresh = if fresh.state == v1::DevelopmentAttemptState::Queued as i32 {
                store
                    .transition_attempt(
                        &fresh.id,
                        fresh.resource_version,
                        v1::DevelopmentAttemptState::Running,
                        None,
                    )
                    .map_err(|_| "coding_state_changed")?
            } else {
                fresh
            };
            store
                .coding_details(&fresh.id, report.requested_input.clone(), usage)
                .map_err(|_| "coding_result_rejected")?;
            let fresh = store
                .development_attempt(&fresh.id)
                .map_err(|_| "coding_state_invalid")?;
            store
                .transition_attempt(&fresh.id, fresh.resource_version, next, None)
                .map_err(|_| "coding_state_changed")?;
            if next == v1::DevelopmentAttemptState::Succeeded {
                store
                    .record_candidate(v1::Candidate {
                        development_attempt_id: fresh.id,
                        repository_connection_id: source.repository_connection_id.clone(),
                        base_revision: source.revision.clone(),
                        identity: Some(v1::CandidateIdentity {
                            identity: Some(v1::candidate_identity::Identity::Git(
                                v1::GitCandidate {
                                    commit_sha: report.commit.ok_or("coding_commit_missing")?,
                                },
                            )),
                        }),
                        state: v1::CandidateState::Available as i32,
                        changed_paths: report.changed_paths,
                        coding_advisory: Some(report.advisory),
                        ..Default::default()
                    })
                    .map_err(|_| "coding_candidate_rejected")?;
            }
        }
        Ok(())
    }

    fn coding_failed(
        &self,
        attempt: &v1::DevelopmentAttempt,
        reason: &str,
    ) -> Result<(), &'static str> {
        self.state
            .store
            .lock()
            .map_err(|_| "runtime_store_unavailable")?
            .transition_attempt(
                &attempt.id,
                attempt.resource_version,
                v1::DevelopmentAttemptState::Failed,
                Some(v1::ResourceFailure {
                    reason: v1::ErrorReason::RuntimeArtifactNotVerified as i32,
                    message: reason.into(),
                    remediation: Some(
                        "Inspect the isolated Job, repair the cause, then retry the task.".into(),
                    ),
                    observed_at: None,
                }),
            )
            .map_err(|_| "coding_state_changed")?;
        Ok(())
    }
}

fn collect_coding_result(
    directory: &std::path::Path,
) -> Result<
    (
        ayni_orchestrator_runtime::coding::CodingReport,
        v1::AttemptModelUsage,
    ),
    &'static str,
> {
    use ayni_orchestrator_runtime::{coding::CodingReport, runtime_execution::read_bounded};
    let report: CodingReport = serde_json::from_slice(
        &read_bounded(&directory.join("report.json"), 65536)
            .map_err(|_| "coding_output_unavailable")?,
    )
    .map_err(|_| "coding_output_invalid")?;
    if report.advisory.len() > 512
        || report
            .requested_input
            .as_ref()
            .is_some_and(|q| q.len() > 1024)
        || report.changed_paths.len() > 256
    {
        return Err("coding_output_invalid");
    }
    if report.requested_input.is_none() {
        if report
            .commit
            .as_ref()
            .is_none_or(|sha| sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("coding_commit_missing");
        }
        let bundle = read_bounded(&directory.join("candidate.bundle"), 64 * 1024 * 1024)
            .map_err(|_| "coding_bundle_unavailable")?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bundle));
        if report.bundle_digest.as_ref() != Some(&digest) {
            return Err("coding_bundle_mismatch");
        }
    }
    Ok((report, collect_coding_usage(directory)?))
}

fn collect_coding_usage(
    directory: &std::path::Path,
) -> Result<v1::AttemptModelUsage, &'static str> {
    use ayni_orchestrator_domain::{ModelInteractionOutcome, ModelUsageReport};
    let mut interactions = Vec::new();
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
fn milliseconds(ms: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: ms.div_euclid(1000),
        nanos: i32::try_from(ms.rem_euclid(1000) * 1_000_000).unwrap_or(0),
    }
}

fn coding_seconds_remaining(snapshot: &v1::DevelopmentSnapshot) -> u64 {
    snapshot.deadline_at.as_ref().map_or(3600, |t| {
        u64::try_from(t.seconds)
            .unwrap_or(0)
            .saturating_sub(now().unwrap_or(u64::MAX))
    })
}


const fn coding_needs_cleanup(state: v1::DevelopmentAttemptState) -> bool {
    matches!(state,
        v1::DevelopmentAttemptState::Cancelled
            | v1::DevelopmentAttemptState::DeadlineExceeded
            | v1::DevelopmentAttemptState::Failed
            | v1::DevelopmentAttemptState::Unsupported
            | v1::DevelopmentAttemptState::WaitingForInput
    )
}
