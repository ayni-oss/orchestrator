//! Ayni-specific durable data and legacy cursor decoding. No coordinator owns persisted types.
use ayni_orchestrator_api::v1;
use ayni_orchestrator_application::execution_settings::ProfileRuntime;
use ayni_orchestrator_application::runtime_preparation::{
    CredentialReferences, ImagePreparationPhase, RetainedCredentials, RuntimePreparation,
};
use ayni_orchestrator_kubernetes::execution::operation_name;
use ayni_orchestrator_profile_ayni::runtime_execution::ExecutionRequest;
use serde::{Deserialize, Serialize};

pub type RuntimeCursor = RuntimePreparation<PendingVerification, PreparationIdentity, TrustUpdate>;

pub trait RuntimeCursorExt {
    fn retry(&mut self);
    fn retry_exhausted(&self) -> bool;
    fn resume_cleanup(&mut self) -> bool;
    fn request_rebuild(&mut self, requested: bool);
    fn update_status(
        &mut self,
        runtime: &ProfileRuntime,
        fingerprint: String,
    ) -> Result<(), &'static str>;
    fn metadata(&self) -> Vec<v1::VerifiedMetadata>;
}
impl RuntimeCursorExt for RuntimeCursor {
    fn retry_exhausted(&self) -> bool {
        self.failures >= 3 || self.accounting.blocked
    }

    fn retry(&mut self) {
        self.next_at = 0;
        self.failures = 0;
        self.accounting.blocked = false;
        self.retry_trust = true;
        self.builder.started_at = None;
        self.status.remove("reason-code");
        self.status.remove("error-class");
        self.status.remove("last-error");
        self.accounting.operational.clear();
        if let Some(pending) = &mut self.pending {
            pending.failed = false;
        }
    }

    // Recover cursors written when asynchronous deletion incorrectly consumed retries.
    fn resume_cleanup(&mut self) -> bool {
        if self.status.get("reason-code").map(String::as_str) != Some("runtime_cleanup_pending") {
            return false;
        }
        self.status.remove("reason-code");
        self.next_at = 0;
        true
    }

    fn request_rebuild(&mut self, requested: bool) {
        self.force_rebuild |= requested;
        self.retry_trust = true;
        self.builder.started_at = None;
    }

    fn update_status(
        &mut self,
        runtime: &ProfileRuntime,
        fingerprint: String,
    ) -> Result<(), &'static str> {
        self.status.insert(
            "source-kind".into(),
            if runtime.build.is_some() {
                "build"
            } else {
                "registry"
            }
            .into(),
        );
        self.status
            .insert("discovery-reference".into(), runtime.image.clone());
        self.status.insert(
            "default-sha".into(),
            self.pending
                .as_ref()
                .ok_or("runtime_self_invalid")?
                .request
                .revision
                .clone(),
        );
        self.status.insert("lock-fingerprint".into(), fingerprint);
        if let Some(name) = self.pending.as_ref().and_then(|p| p.build_name.clone()) {
            self.status.insert("build-job".into(), name);
        } else {
            self.status.remove("build-job");
        }
        Ok(())
    }

    fn metadata(&self) -> Vec<v1::VerifiedMetadata> {
        let mut status = self.status.clone();
        if let Some(preparation) = &self.preparation {
            status.insert(
                "verification-authority".into(),
                preparation.verification_authority.code().into(),
            );
        }
        // Older cursors could persist verification before updating display metadata
        // when asynchronous Job cleanup interrupted reconciliation.
        if self
            .trust
            .as_ref()
            .is_some_and(|trust| trust.state == "verified")
        {
            status.insert("certificate-trust-state".into(), "verified".into());
        }
        if let Some(image) = &self.builder.image {
            status.insert("builder-image".into(), image.clone());
        }
        status.insert(
            "retry-eligibility".into(),
            if self.retry_exhausted() {
                "operator_required"
            } else {
                "automatic"
            }
            .into(),
        );
        status.insert(
            "failed-executions".into(),
            self.accounting.executions.len().to_string(),
        );
        if !self.retry_exhausted() && self.next_at > 0 {
            status.insert("next-retry".into(), self.next_at.to_string());
        } else {
            status.remove("next-retry");
        }
        if self.accounting.legacy_failures > 0 {
            status.insert(
                "legacy-failures".into(),
                self.accounting.legacy_failures.to_string(),
            );
        }
        if let Some(last) = self.last_success {
            status.insert("last-success".into(), last.to_string());
        }
        status
            .into_iter()
            .map(|(key, value)| v1::VerifiedMetadata { key, value })
            .collect()
    }
}

/// Complete public input identity retained before any repository preparation Job.
#[derive(Clone, Deserialize, Serialize)]
pub struct PreparationIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_reporting:
        Option<ayni_orchestrator_runtime::verification::ReportingConfiguration>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub operation_observations:
        std::collections::BTreeMap<String, ayni_orchestrator_events::EventIdentity>,
    #[serde(default)]
    pub verification_authority: VerificationAuthority,
    pub generation: String,
    pub source_revision: String,
    pub policy_digest: String,
    pub lock_digest: String,
    pub contract_version: String,
    pub platform: String,
    pub registry: Vec<u8>,
    pub certificate: Option<SigningIdentity>,
    pub runtime: ProfileRuntime,
}

/// Frozen per preparation; operator defaults never reinterpret admitted work.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerificationAuthority {
    #[default]
    Retained,
    Shadow {
        reporting: ayni_orchestrator_runtime::verification::ReportingConfiguration,
    },
    Events {
        reporting: ayni_orchestrator_runtime::verification::ReportingConfiguration,
    },
}
impl VerificationAuthority {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Retained => "retained",
            Self::Shadow { .. } => "shadow",
            Self::Events { .. } => "events",
        }
    }
    pub const fn events(&self) -> bool {
        matches!(self, Self::Events { .. })
    }
    pub const fn reporting(
        &self,
    ) -> Option<&ayni_orchestrator_runtime::verification::ReportingConfiguration> {
        match self {
            Self::Retained => None,
            Self::Shadow { reporting } | Self::Events { reporting } => Some(reporting),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct PendingVerification {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ayni_orchestrator_events::EventIdentity>,
    pub name: String,
    pub profile_revision: String,
    pub request: ExecutionRequest,
    pub image: String,
    #[serde(default)]
    pub frozen_runtime: Option<ProfileRuntime>,
    pub started_at: u64,
    pub source_digest: String,
    pub lock_digest: String,
    pub build_name: Option<String>,
    pub phase: ImagePreparationPhase,
    #[serde(default)]
    pub candidate_image_id: Option<String>,
    #[serde(default)]
    pub publish_name: Option<String>,
    #[serde(default)]
    pub publish_attempts: u32,
    #[serde(default = "publication_budget")]
    pub publish_remaining: u8,
    #[serde(default)]
    pub candidate_missing: bool,
    #[serde(default)]
    pub force_rebuild: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate: Option<SigningIdentity>,

    #[serde(default)]
    pub resolved_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
}

pub const fn publication_budget() -> u8 {
    3
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SigningIdentity {
    pub id: String,
    pub revision: String,
    pub key_id: String,
    pub public_key: String,
    pub secret_name: String,
    pub enabled: bool,
    pub status: String,
}
impl From<v1::Certificate> for SigningIdentity {
    fn from(certificate: v1::Certificate) -> Self {
        Self {
            id: certificate.id,
            revision: certificate.revision,
            key_id: certificate.key_id,
            public_key: certificate.public_key,
            secret_name: certificate.secret_name,
            enabled: certificate.enabled,
            status: certificate.status,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct TrustUpdate {
    pub generation: String,
    pub head: String,
    pub job: String,
    #[serde(default)]
    pub attempt: u64,
    pub result: Option<TrustResult>,
    pub pull_number: Option<u64>,
    pub state: String,
    #[serde(default)]
    pub merge_revision: Option<String>,
}
impl TrustUpdate {
    pub fn reset(&mut self, head: &str) {
        self.attempt = self.attempt.saturating_add(1);
        self.head = head.into();
        self.job = operation_name(&format!("trust:{}:{head}", self.generation), self.attempt);
        self.result = None;
        self.pull_number = None;
        self.state = "pending".into();
    }

    pub fn branch() -> String {
        "setup/certificate-trust".into()
    }

    pub fn observe_pull(
        &mut self,
        merged: bool,
        open: bool,
        pinned: bool,
        head: &str,
    ) -> Result<(), &'static str> {
        if merged {
            if !pinned {
                return Err("certificate_trust_merge_mismatch");
            }
            self.reset(head);
        } else {
            self.state = if open { "open" } else { "closed" }.into();
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TrustResult {
    pub revision: String,
    pub policy: String,
    pub lock: String,
}

#[derive(Deserialize)]
struct PendingWire {
    #[serde(flatten)]
    legacy: LegacyProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ayni_orchestrator_events::EventIdentity>,
    pub name: String,
    pub profile_revision: String,
    pub request: ExecutionRequest,
    pub image: String,
    #[serde(default)]
    pub frozen_runtime: Option<ProfileRuntime>,
    pub started_at: u64,
    pub source_digest: String,
    pub lock_digest: String,
    pub build_name: Option<String>,
    #[serde(default)]
    pub phase: Option<ImagePreparationPhase>,
    #[serde(default)]
    pub candidate_image_id: Option<String>,
    #[serde(default)]
    pub publish_name: Option<String>,
    #[serde(default)]
    pub publish_attempts: u32,
    #[serde(default = "publication_budget")]
    pub publish_remaining: u8,
    #[serde(default)]
    pub candidate_missing: bool,
    #[serde(default)]
    pub force_rebuild: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate: Option<SigningIdentity>,

    #[serde(default)]
    pub resolved_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ayni_orchestrator_profile_ayni::runtime_execution::VerificationReport>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
}

#[derive(Default, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "schema9 compatibility codec must read the exact persisted boolean fields"
)]
struct LegacyProgress {
    #[serde(default)]
    build_started: bool,
    #[serde(default)]
    build_completed: bool,
    #[serde(default)]
    build_observed: bool,
    #[serde(default)]
    discovery_started: bool,
    #[serde(default)]
    discovery_complete: bool,
}

impl<'de> Deserialize<'de> for PendingVerification {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = PendingWire::deserialize(deserializer)?;
        let phase = if let Some(phase) = wire.phase {
            phase
        } else {
            let progress = &wire.legacy;
            if wire.verification.is_some() {
                ImagePreparationPhase::Activating
            } else if !progress.discovery_complete {
                match (progress.build_observed, progress.discovery_started) {
                    (true, true) => ImagePreparationPhase::PublicationReadbackStarted,
                    (true, false) => ImagePreparationPhase::PublicationReadback,
                    (false, true) => ImagePreparationPhase::DiscoveringStarted,
                    (false, false) => ImagePreparationPhase::Discovering,
                }
            } else if wire.build_name.is_some() && !progress.build_observed {
                if progress.build_completed {
                    ImagePreparationPhase::Publishing
                } else if progress.build_started {
                    ImagePreparationPhase::BuildingStarted
                } else {
                    ImagePreparationPhase::Building
                }
            } else {
                ImagePreparationPhase::Verifying
            }
        };
        if matches!(
            phase,
            ImagePreparationPhase::Publishing
                | ImagePreparationPhase::PublicationReadback
                | ImagePreparationPhase::PublicationReadbackStarted
        ) && (wire.build_name.is_none() || wire.candidate_image_id.is_none())
        {
            return Err(serde::de::Error::custom(
                "publication requires a frozen build and candidate",
            ));
        }
        if phase == ImagePreparationPhase::Verifying && wire.resolved_digest.is_none() {
            return Err(serde::de::Error::custom(
                "verification requires a resolved digest",
            ));
        }
        if phase == ImagePreparationPhase::Activating
            && (wire.verification.is_none() || wire.resolved_digest.is_none())
        {
            return Err(serde::de::Error::custom(
                "activation requires retained verification evidence",
            ));
        }
        Ok(Self {
            phase,
            observation: wire.observation,
            name: wire.name,
            profile_revision: wire.profile_revision,
            request: wire.request,
            image: wire.image,
            frozen_runtime: wire.frozen_runtime,
            started_at: wire.started_at,
            source_digest: wire.source_digest,
            lock_digest: wire.lock_digest,
            build_name: wire.build_name,
            candidate_image_id: wire.candidate_image_id,
            publish_name: wire.publish_name,
            publish_attempts: wire.publish_attempts,
            publish_remaining: wire.publish_remaining,
            candidate_missing: wire.candidate_missing,
            force_rebuild: wire.force_rebuild,
            certificate: wire.certificate,
            resolved_digest: wire.resolved_digest,
            verification: wire.verification,
            failed: wire.failed,
        })
    }
}

impl RetainedCredentials for PendingVerification {
    fn credential_references(&self) -> CredentialReferences {
        credentials(self.frozen_runtime.as_ref(), self.certificate.as_ref())
    }
}
impl RetainedCredentials for PreparationIdentity {
    fn credential_references(&self) -> CredentialReferences {
        credentials(Some(&self.runtime), self.certificate.as_ref())
    }
}
impl RetainedCredentials for TrustUpdate {
    fn credential_references(&self) -> CredentialReferences {
        CredentialReferences::default()
    }
}
fn credentials(
    runtime: Option<&ProfileRuntime>,
    certificate: Option<&SigningIdentity>,
) -> CredentialReferences {
    let mut refs = CredentialReferences::default();
    if let Some(runtime) = runtime {
        refs.secrets.extend(runtime.pull_secret.iter().cloned());
        if let Some(build) = &runtime.build {
            refs.secrets.extend(
                [
                    &build.registry_secret,
                    &build.signing_secret,
                    &build.engine_secret,
                ]
                .into_iter()
                .filter(|name| !name.is_empty())
                .cloned(),
            );
        }
    }
    if let Some(certificate) = certificate {
        refs.certificates.insert(certificate.id.clone());
        refs.secrets.insert(certificate.secret_name.clone());
    }
    refs
}

#[cfg(test)]
pub mod tests;
