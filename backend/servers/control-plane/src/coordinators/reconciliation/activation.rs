//! Restartable verification evidence, cleanup, and guarded runtime activation.
use super::{
    Digest, ExecutionBackend, ProfileObservation, RuntimeCursor, RuntimeReconciler, Sha256,
};

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn accept(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        digest: String,
        report: &ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
    ) -> Result<(), &'static str> {
        self.accept_report(observation, cursor, digest, None, report)
            .await
    }

    pub(super) async fn accept_report(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        digest: String,
        receipt: Option<ayni_orchestrator_events::EventIdentity>,
        report: &ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
    ) -> Result<(), &'static str> {
        use tracing::Instrument;
        let execution = cursor
            .pending
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let span = self.verification_span("acceptance", &execution).await;
        let result = self
            .accept_report_inner(observation, cursor, digest, receipt, report)
            .instrument(span.clone())
            .await;
        if let Err(reason) = result {
            ayni_orchestrator_observability::record_failure(&span, reason);
        }
        result
    }

    #[allow(
        clippy::too_many_lines,
        reason = "evidence admission, durable cleanup and guarded activation are one ordered transaction protocol"
    )]
    pub(super) async fn accept_report_inner(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        digest: String,
        receipt: Option<ayni_orchestrator_events::EventIdentity>,
        report: &ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let github = &observation.github;
        let head = observation.head.as_str();
        let token = observation.token.as_str();
        let source_digest = observation.source_digest.as_str();
        let expected_cursor =
            serde_json::to_string(cursor).map_err(|_| "runtime_cursor_invalid")?;
        let pending = cursor.pending.clone().ok_or("runtime_cursor_invalid")?;
        if let Some(certificate) = &pending.certificate {
            if report.runtime.certificate_key_id != certificate.key_id {
                return Err("certificate_verification_mismatch");
            }
        } else {
            return Err("certificate_setup_required");
        }
        crate::profiles::ayni::evidence_policy::admit_runtime(
            report,
            &pending.request.expected,
            head,
            &pending.lock_digest,
        )
        .map_err(crate::profiles::ayni::evidence_policy::EvidenceError::code)?;
        if self
            .source
            .head(
                &github.owner,
                &github.name,
                &observation.default_branch,
                token,
            )
            .await?
            != head
        {
            return Err("runtime_source_changed");
        }
        if pending.resolved_digest.as_deref() != Some(digest.as_str()) {
            return Err("runtime_digest_mismatch");
        }
        if let Some((_, expected)) = runtime.image.split_once('@')
            && expected != digest
        {
            return Err("runtime_digest_mismatch");
        }
        let key = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}:{}:{}",
                source_digest, runtime.platform, report.runtime.lock_fingerprint
            ))
        );
        if !pending.force_rebuild
            && cursor
                .bindings
                .get(&key)
                .is_some_and(|bound| bound != &digest)
        {
            return Err("runtime_tag_drift");
        }
        if cursor.bindings.len() >= 128 && !cursor.bindings.contains_key(&key) {
            return Err("runtime_binding_limit");
        }
        cursor.bindings.insert(key, digest.clone());
        cursor
            .status
            .insert("resolved-digest".into(), digest.clone());

        let command = activation_command(id, runtime, cursor, &digest, report)?;
        // Retain evidence before cleanup so a crash cannot discard admission authority.
        cursor
            .pending
            .as_mut()
            .ok_or("runtime_cursor_invalid")?
            .verification = Some(report.clone());
        cursor
            .pending
            .as_mut()
            .ok_or("runtime_cursor_invalid")?
            .phase =
            ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase::Activating;
        if let Some(receipt) = receipt {
            cursor
                .pending
                .as_mut()
                .ok_or("runtime_cursor_invalid")?
                .observation = Some(receipt);
        }
        cursor.status.insert("phase".into(), "cleanup".into());
        let next = cursor.clone();
        let cleanup_profile = id.to_owned();
        let cleanup_expected = self
            .with_database(move |store| {
                store
                    .commit_verification(&command, &expected_cursor, &next)
                    .map_err(|_| "runtime_activation_rejected")?;
                serde_json::to_string(
                    &store
                        .runtime_cursor(&cleanup_profile)
                        .map_err(|_| "runtime_cursor_invalid")?,
                )
                .map_err(|_| "runtime_cursor_invalid")
            })
            .await?;
        self.finish_verification_cleanup(observation.id, observation.time, cursor, cleanup_expected)
            .await
    }

    pub(super) async fn resume_accepted_cleanup(
        &self,
        id: &str,
        time: u64,
        cursor: &mut RuntimeCursor,
    ) -> Result<bool, &'static str> {
        if cursor
            .pending
            .as_ref()
            .is_none_or(|p| p.verification.is_none())
        {
            return Ok(false);
        }
        let profile = id.to_owned();
        let snapshot = cursor.clone();
        let committed = self
            .with_database(move |store| {
                store
                    .verification_cleanup_committed(&profile, &snapshot)
                    .map_err(|_| "runtime_cursor_invalid")
            })
            .await?;
        if !committed {
            return Ok(false);
        }
        let expected = serde_json::to_string(cursor).map_err(|_| "runtime_cursor_invalid")?;
        self.finish_verification_cleanup(id, time, cursor, expected)
            .await?;
        Ok(true)
    }

    pub(super) async fn finish_verification_cleanup(
        &self,
        id: &str,
        time: u64,
        cursor: &mut RuntimeCursor,
        cleanup_expected: String,
    ) -> Result<(), &'static str> {
        let pending = cursor.pending.clone().ok_or("runtime_cursor_invalid")?;
        self.cleanup_operation(&pending).await?;
        cursor.pending = None;
        cursor.preparation = None;
        cursor.last_success = Some(time);
        cursor.failures = 0;
        cursor.accounting.blocked = false;
        cursor.accounting.operational.clear();
        cursor.status.remove("error-class");
        cursor.status.remove("last-error");
        cursor.status.remove("reason-code");
        cursor.status.insert("phase".into(), "ready".into());
        let jitter =
            u64::from(Sha256::digest(id)[0]) % (self.configuration.interval_seconds / 5 + 1);
        cursor.next_at = time + self.configuration.interval_seconds + jitter;
        let profile = id.to_owned();
        let completed = cursor.clone();
        self.with_database(move |store| {
            store
                .complete_verification_cleanup(&profile, &cleanup_expected, &completed)
                .map_err(|_| "runtime_cleanup_changed")
        })
        .await?;
        Ok(())
    }
}

fn activation_command(
    id: &str,
    runtime: &super::ProfileRuntime,
    cursor: &RuntimeCursor,
    digest: &str,
    report: &ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
) -> Result<ayni_orchestrator_application::runtime_activation::ActivateRuntime, &'static str> {
    let pending = cursor.pending.as_ref().ok_or("runtime_cursor_invalid")?;
    let mut command = crate::profiles::ayni::runtime_activation::command(
        id,
        &pending.profile_revision,
        &runtime.image,
        digest,
        report,
    )
    .map_err(|_| "runtime_activation_rejected")?;
    let identity = cursor
        .preparation
        .as_ref()
        .ok_or("runtime_preparation_missing")?;
    if identity.source_revision != pending.request.revision
        || identity.lock_digest != pending.lock_digest
    {
        return Err("runtime_preparation_mismatch");
    }
    let certificate = pending
        .certificate
        .as_ref()
        .ok_or("certificate_setup_required")?;
    let builder = pending
        .frozen_runtime
        .as_ref()
        .and_then(|runtime| runtime.build.as_ref())
        .ok_or("runtime_builder_missing")?;
    let contract =
        ayni_orchestrator_profile_ayni::compatibility::contract(&identity.contract_version)?;
    command.artifact.metadata.extend([
        ("ayni-contract".into(), identity.contract_version.clone()),
        ("contract-lock-schema".into(), contract.lock_schema.into()),
        ("contract-recipe".into(), contract.recipe_version.into()),
        (
            "contract-metadata-schema".into(),
            contract.metadata_schema.into(),
        ),
        (
            "contract-signals-schema".into(),
            contract.signals_schema.into(),
        ),
        ("builder-image".into(), builder.image.clone()),
        ("source-revision".into(), identity.source_revision.clone()),
        (
            "source-policy-digest".into(),
            identity.policy_digest.clone(),
        ),
        ("preparation-generation".into(), identity.generation.clone()),
        ("certificate-id".into(), certificate.id.clone()),
        ("certificate-revision".into(), certificate.revision.clone()),
    ]);
    Ok(command)
}
