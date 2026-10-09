//! Registry discovery, isolated builds and automatic publication.
use super::{
    ExecutionBackend, ImageDiscovery, Observation, PendingPhase, PendingVerification,
    ProfileObservation, RuntimeCursor, RuntimeReconciler, VerificationAction, discovery_error,
    operation_name, verification_action,
};
use ayni_orchestrator_application::execution::ExecutionState;
use ayni_orchestrator_application::image_build::{BuildObservation, observe_build};

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn observe_pending(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let time = observation.time;

        let pending = cursor.pending.clone().ok_or("runtime_cursor_invalid")?;
        let phase = self
            .pending_verification_phase(observation, &pending)
            .await?;
        if phase == PendingPhase::DiscardStale {
            self.cleanup_operation(&pending).await?;
            cursor.pending = None;
            cursor.status.remove("reason-code");
            cursor.status.insert("phase".into(), "analyzing".into());
            self.save(id, cursor).await?;
            return Ok(());
        }
        if pending.failed {
            // Resume cleanup after a restart without recreating failed Jobs.
            return self.cleanup_operation(&pending).await;
        }
        if let Some(report) = &pending.verification {
            return self
                .accept(
                    observation,
                    cursor,
                    pending
                        .resolved_digest
                        .clone()
                        .ok_or("runtime_digest_unavailable")?,
                    report,
                )
                .await;
        }
        if !pending.discovery.complete {
            return self.observe_discovery(observation, cursor, &pending).await;
        }
        if let Some(build_name) = &pending.build_name
            && phase == PendingPhase::ObserveBuild
        {
            self.observe_build(observation, cursor, &pending, build_name)
                .await?;
            return Ok(());
        }
        let observation_result = self.kubernetes.observe(&pending.name).await?;
        match verification_action(&observation_result, time.saturating_sub(pending.started_at)) {
            VerificationAction::Start => {
                self.start_verification(observation, &pending).await?;
            }
            VerificationAction::Wait => {}
            VerificationAction::Timeout => return Err("runtime_verification_timeout"),
            VerificationAction::Failure => {
                let Observation::Failed(reason) = observation_result else {
                    return Err("runtime_result_invalid");
                };
                return Err(reason);
            }
            VerificationAction::Accept => {
                let Observation::Completed { digest, report } = observation_result else {
                    return Err("runtime_result_invalid");
                };
                let report = serde_json::from_str(&report).map_err(|_| "runtime_result_invalid")?;
                self.accept(observation, cursor, digest, &report).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn record_failure(
        &self,
        id: &str,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        crate::process::metrics::record_runtime_failure("build", reason);
        let id = id.to_owned();
        let interval = self.configuration.interval_seconds;
        let pending = self
            .with_database(move |store| {
                store
                    .runtime_failure(&id, reason)
                    .map_err(|_| "runtime_cursor_invalid")?;
                let mut cursor = store
                    .runtime_cursor(&id)
                    .map_err(|_| "runtime_cursor_invalid")?;
                cursor.failures = cursor.failures.saturating_add(1);
                cursor.status.insert("reason-code".into(), reason.into());
                cursor.next_at = super::now()? + super::retry_delay(interval, cursor.failures);
                if cursor.failures >= 3
                    && let Some(pending) = &mut cursor.pending
                {
                    pending.failed = true;
                }
                store
                    .save_runtime_cursor(&id, &cursor)
                    .map_err(|_| "runtime_cursor_invalid")?;
                Ok(cursor.pending.filter(|pending| pending.failed))
            })
            .await?;
        if let Some(pending) = pending {
            self.cleanup_operation(&pending).await?;
        }
        Ok(())
    }

    pub(super) async fn cleanup_exhausted_cursor(
        &self,
        cursor: &RuntimeCursor,
    ) -> Result<(), &'static str> {
        if let Some(pending) = &cursor.pending {
            self.cleanup_operation(pending).await?;
        }
        if let Some(name) = cursor.status.get("builder-job") {
            self.kubernetes.cleanup(name).await?;
        }
        if let Some(trust) = &cursor.trust {
            self.kubernetes.cleanup(&trust.job).await?;
        }
        Ok(())
    }

    pub(super) async fn cleanup_operation(
        &self,
        pending: &PendingVerification,
    ) -> Result<(), &'static str> {
        let discovery = operation_name(
            &format!("discover:{}", pending.name),
            u64::from(pending.publish_attempts),
        );
        for name in [
            Some(pending.name.as_str()),
            pending.publish_name.as_deref(),
            pending.build_name.as_deref(),
            Some(discovery.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            self.kubernetes.cleanup(name).await?;
        }
        Ok(())
    }

    async fn start_verification(
        &self,
        observation: &ProfileObservation<'_>,
        pending: &PendingVerification,
    ) -> Result<(), &'static str> {
        let runtime = observation.runtime;
        let token = observation.token.as_str();
        let mut verification_runtime = runtime.execution_runtime();
        let repository =
            ayni_orchestrator_application::execution_settings::image_repository(&runtime.image)
                .ok_or("runtime_image_invalid")?;
        verification_runtime.image = format!(
            "{repository}@{}",
            pending
                .resolved_digest
                .as_deref()
                .ok_or("runtime_digest_unavailable")?
        );
        verification_runtime.build = None;
        self.kubernetes
            .start(
                &pending.name,
                &verification_runtime,
                &pending.request,
                token,
            )
            .await?;
        Ok(())
    }

    async fn observe_discovery(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        pending: &PendingVerification,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let time = observation.time;
        let discovery_name = operation_name(
            &format!("discover:{}", pending.name),
            u64::from(pending.publish_attempts),
        );
        if !pending.discovery.started {
            cursor
                .pending
                .as_mut()
                .ok_or("runtime_cursor_invalid")?
                .discovery
                .started = true;
            cursor.status.insert("phase".into(), "analyzing".into());
            self.save(id, cursor).await?;
        }
        // Creation is idempotent: adopt a Job after a controller crash.
        self.kubernetes
            .start_discovery(&discovery_name, &runtime.execution_runtime())
            .await?;
        let discovery = self.kubernetes.discover(&discovery_name).await?;
        let discovery = publication_readback(
            discovery,
            pending
                .candidate_image_id
                .as_deref()
                .filter(|_| pending.build.observed),
            pending.force_rebuild,
        )?;
        match discovery {
            ImageDiscovery::Pending => {
                if time.saturating_sub(pending.started_at) > 180 {
                    return Err("registry_discovery_timeout");
                }
                return Ok(());
            }
            ImageDiscovery::Missing => {
                if pending.build.observed {
                    return self
                        .retry_publication_readback(id, cursor, pending, &discovery_name)
                        .await;
                }
                if runtime.build.is_none() {
                    return Err("runtime_builder_missing");
                }
                let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
                next.discovery.complete = true;
                next.build_name = Some(operation_name(&format!("build:{}", pending.name), 0));
                cursor.status.insert("phase".into(), "building".into());
            }
            ImageDiscovery::Resolved { digest, .. } => {
                let repository =
                    ayni_orchestrator_application::execution_settings::image_repository(
                        &runtime.image,
                    )
                    .ok_or("runtime_image_invalid")?;
                ayni_orchestrator_runtime::runtime_contract::RuntimeImage::parse(&format!(
                    "{repository}@{digest}"
                ))
                .map_err(|_| "runtime_digest_unavailable")?;
                let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
                next.discovery.complete = true;
                if pending.force_rebuild && !pending.build.observed {
                    if runtime.build.is_none() {
                        return Err("runtime_builder_missing");
                    }
                    next.build_name = Some(operation_name(&format!("build:{}", pending.name), 0));
                    cursor.status.insert("phase".into(), "building".into());
                } else {
                    next.resolved_digest = Some(digest);
                    cursor.status.insert("phase".into(), "verifying".into());
                }
            }
            ImageDiscovery::Failed { reason }
                if pending.force_rebuild
                    && matches!(
                        reason.as_str(),
                        "runtime_platform_mismatch" | "runtime_digest_mismatch"
                    ) =>
            {
                if runtime.build.is_none() {
                    return Err("runtime_builder_missing");
                }
                let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
                next.discovery.complete = true;
                next.build_name = Some(operation_name(&format!("build:{}", pending.name), 0));
                cursor.status.insert("phase".into(), "building".into());
            }
            ImageDiscovery::Failed { reason } => return Err(discovery_error(&reason)),
        }
        self.save(id, cursor).await?;
        self.kubernetes.cleanup(&discovery_name).await?;
        Ok(())
    }

    async fn retry_publication_readback(
        &self,
        id: &str,
        cursor: &mut RuntimeCursor,
        pending: &PendingVerification,
        discovery_name: &str,
    ) -> Result<(), &'static str> {
        if pending.publish_remaining == 0 {
            return Err("runtime_publication_failed");
        }
        let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
        next.build.observed = false;
        next.publish_name = None;
        next.discovery.complete = true;
        if pending.candidate_missing {
            next.build.completed = false;
            next.build.started = false;
            next.candidate_image_id = None;
            next.candidate_missing = false;
            next.build_name = Some(operation_name(
                &format!("build:{}", pending.name),
                u64::from(pending.publish_attempts),
            ));
            cursor.status.insert("phase".into(), "building".into());
        } else {
            cursor.status.insert("phase".into(), "publishing".into());
        }
        self.save(id, cursor).await?;
        if pending.candidate_missing
            && let Some(name) = &pending.build_name
        {
            self.kubernetes.cleanup(name).await?;
        }
        self.kubernetes.cleanup(discovery_name).await?;
        Ok(())
    }

    pub(super) async fn observe_build(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        pending: &PendingVerification,
        build_name: &str,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let token = observation.token.as_str();
        let build_state = observe_build(
            &self.kubernetes,
            build_name,
            &runtime.execution_runtime(),
            pending.build.started,
        )
        .await?;
        if matches!(build_state, BuildObservation::EngineLost) {
            // Cleanup is retried against the old durable identity before advancing it.
            self.cleanup_operation(pending).await?;
            let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
            restart_engine_attempt(next)?;
            cursor.status.insert("phase".into(), "building".into());
            self.save(id, cursor).await?;
            return Ok(());
        }
        if !pending.build.completed {
            match build_state {
                BuildObservation::Missing => {
                    cursor
                        .pending
                        .as_mut()
                        .ok_or("runtime_cursor_invalid")?
                        .build
                        .started = true;
                    cursor.status.insert("phase".into(), "building".into());
                    self.save(id, cursor).await?;
                    self.kubernetes
                        .start_image_build(
                            build_name,
                            &runtime.execution_runtime(),
                            &pending.request,
                            token,
                            &ayni_orchestrator_application::image_build::ImageBuildKind::RepositoryRuntime,
                        )
                        .await?;
                }
                BuildObservation::Running => {}
                BuildObservation::Completed(image_id) => {
                    let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
                    next.build.completed = true;
                    next.candidate_image_id = Some(image_id);
                    cursor.status.insert("phase".into(), "publishing".into());
                    self.save(id, cursor).await?;
                }
                BuildObservation::Failed(code) => return Err(build_error(&code)),
                BuildObservation::EngineLost => return Err("engine_lost"),
            }
            return Ok(());
        }
        self.observe_publication(observation, cursor, pending).await
    }

    async fn observe_publication(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        pending: &PendingVerification,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let time = observation.time;
        let image_id = pending
            .candidate_image_id
            .as_deref()
            .ok_or("runtime_build_result_missing")?;
        let publish_name = if let Some(name) = &pending.publish_name {
            name.clone()
        } else {
            let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
            next.publish_attempts = next
                .publish_attempts
                .checked_add(1)
                .ok_or("runtime_sequence_exhausted")?;
            next.publish_remaining = next
                .publish_remaining
                .checked_sub(1)
                .ok_or("runtime_publication_failed")?;
            let name = operation_name(
                &format!("publish:{}", pending.name),
                u64::from(next.publish_attempts),
            );
            next.publish_name = Some(name.clone());
            cursor.status.insert("phase".into(), "publishing".into());
            self.save(id, cursor).await?;
            name
        };
        match self.kubernetes.coding_state(&publish_name).await? {
            ExecutionState::Missing if pending.publish_name.is_some() => {
                // A persisted publisher may have pushed before its Job disappeared.
                let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
                next.build.observed = true;
                next.discovery.complete = false;
                next.discovery.started = false;
                next.started_at = time;
                self.save(id, cursor).await?;
                self.kubernetes.cleanup(&publish_name).await?;
            }
            ExecutionState::Missing => {
                self.kubernetes
                    .start_publish(
                        &publish_name,
                        &ayni_orchestrator_kubernetes::engine::bind(
                            &runtime.execution_runtime(),
                            pending
                                .build_name
                                .as_deref()
                                .ok_or("runtime_build_result_missing")?,
                        ),
                        &pending.request,
                        image_id,
                        pending.force_rebuild,
                    )
                    .await?;
            }
            ExecutionState::Running => {}
            state @ (ExecutionState::Succeeded | ExecutionState::Failed) => {
                let candidate_missing = state == ExecutionState::Failed
                    && self
                        .kubernetes
                        .operation_failure(&publish_name, "publish")
                        .await?
                        == "runtime_candidate_missing";
                let next = cursor.pending.as_mut().ok_or("runtime_cursor_invalid")?;
                next.candidate_missing = candidate_missing;
                next.build.observed = true;
                next.discovery.complete = false;
                next.discovery.started = false;
                next.started_at = time;
                // Always read back the registry after an uncertain publication.
                self.save(id, cursor).await?;
                self.kubernetes.cleanup(&publish_name).await?;
            }
        }
        Ok(())
    }
}

fn restart_engine_attempt(pending: &mut PendingVerification) -> Result<(), &'static str> {
    if pending.publish_remaining == 0 {
        return Err("engine_recovery_exhausted");
    }
    pending.publish_remaining -= 1;
    pending.publish_attempts = pending
        .publish_attempts
        .checked_add(1)
        .ok_or("runtime_sequence_exhausted")?;
    pending.build_name = Some(operation_name(
        &format!("build:{}", pending.name),
        u64::from(pending.publish_attempts),
    ));
    pending.publish_name = None;
    pending.candidate_image_id = None;
    pending.candidate_missing = false;
    pending.build.started = false;
    pending.build.completed = false;
    pending.build.observed = false;
    Ok(())
}

fn build_error(code: &str) -> &'static str {
    match code {
        "runtime_lock_mismatch" => "runtime_lock_mismatch",
        "engine_tls_or_startup_failed" => "engine_tls_or_startup_failed",
        "runtime_version_unsupported" => "runtime_version_unsupported",
        "runtime_platform_mismatch" => "runtime_platform_mismatch",
        _ => "runtime_build_failed",
    }
}

use ayni_orchestrator_application::image_build::publication_readback;

#[cfg(test)]
mod tests {
    use super::publication_readback;
    use ayni_orchestrator_application::execution::ImageDiscovery;

    fn resolved() -> ImageDiscovery {
        ImageDiscovery::Resolved {
            digest: format!("sha256:{}", "a".repeat(64)),
            image_id: format!("sha256:{}", "b".repeat(64)),
        }
    }

    #[test]
    fn reuse_and_uncertain_publication_require_the_expected_candidate() -> Result<(), &'static str>
    {
        assert!(matches!(
            publication_readback(resolved(), None, false)?,
            ImageDiscovery::Resolved { .. }
        ));
        assert!(matches!(
            publication_readback(
                resolved(),
                Some(&format!("sha256:{}", "b".repeat(64))),
                false
            )?,
            ImageDiscovery::Resolved { .. }
        ));
        assert!(matches!(
            publication_readback(resolved(), Some("different"), false),
            Err("runtime_tag_drift")
        ));
        assert!(matches!(
            publication_readback(resolved(), Some("different"), true)?,
            ImageDiscovery::Missing
        ));
        assert!(matches!(
            publication_readback(
                ImageDiscovery::Failed {
                    reason: "registry_access_denied".into()
                },
                Some("different"),
                true
            )?,
            ImageDiscovery::Failed { .. }
        ));
        Ok(())
    }
}
