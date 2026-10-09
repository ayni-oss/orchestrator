//! Restartable verification evidence, cleanup, and guarded runtime activation.
use super::{
    Digest, ExecutionBackend, ProfileObservation, RuntimeCursor, RuntimeReconciler, Sha256,
};

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    #[allow(
        clippy::too_many_lines,
        reason = "evidence admission, durable cleanup and guarded activation are one ordered transaction protocol"
    )]
    pub(super) async fn accept(
        &self,
        observation: &ProfileObservation<'_>,
        cursor: &mut RuntimeCursor,
        digest: String,
        report: &ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport,
    ) -> Result<(), &'static str> {
        let id = observation.id;
        let runtime = observation.runtime;
        let time = observation.time;
        let github = &observation.github;
        let head = observation.head.as_str();
        let token = observation.token.as_str();
        let source_digest = observation.source_digest.as_str();
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
        // Retain the immutable binding before activation; retries can only replay it.
        self.save(id, cursor).await?;
        let mut command = crate::profiles::ayni::runtime_activation::command(
            id,
            &pending.profile_revision,
            &runtime.image,
            &digest,
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
        // Retain evidence before cleanup so a crash cannot discard admission authority.
        cursor
            .pending
            .as_mut()
            .ok_or("runtime_cursor_invalid")?
            .verification = Some(report.clone());
        cursor.status.insert("phase".into(), "cleanup".into());
        self.save(id, cursor).await?;
        self.cleanup_operation(&pending).await?;
        self.with_database(move |store| {
            ayni_orchestrator_application::runtime_activation::activate(store, &command)
                .map_err(crate::adapters::sqlite::runtime_activation::activation_error)
                .map_err(|_| "runtime_activation_rejected")
        })
        .await?;
        cursor.pending = None;
        cursor.last_success = Some(time);
        cursor.failures = 0;
        cursor.status.remove("reason-code");
        cursor.status.insert("phase".into(), "ready".into());
        let jitter =
            u64::from(Sha256::digest(id)[0]) % (self.configuration.interval_seconds / 5 + 1);
        cursor.next_at = time + self.configuration.interval_seconds + jitter;
        self.save(id, cursor).await?;
        Ok(())
    }
}
