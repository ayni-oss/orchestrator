//! Application runtime policy and its operator-facing configuration contract.
use ayni_orchestrator_application::execution_settings::{
    BuildRuntime, WorkloadRuntime, image_repository,
};
use ayni_orchestrator_kubernetes::configuration::ExecutionConfiguration;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfiguration {
    pub namespace: String,
    pub worker_image: String,
    pub output_claim: String,
    pub model_secret: String,
    #[serde(default = "interval")]
    pub interval_seconds: u64,
    #[serde(default = "timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default = "concurrency")]
    pub concurrency: usize,
    /// Local Docker Desktop only; production keeps platform node selection.
    #[serde(default)]
    pub allow_emulation: bool,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileRuntime>,
    /// Deployment-wide execution policy for App-authorized Ayni Managed
    /// repositories. Their environment image is derived from GitHub identity
    /// and the committed lock rather than configured repository by repository.
    #[serde(default)]
    pub managed_runtime: Option<ManagedRuntime>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRuntime {
    pub image: String,
    pub platform: String,
    #[serde(default = "runtime_account")]
    pub service_account: String,
    #[serde(default)]
    pub pull_secret: Option<String>,
    #[serde(default)]
    pub build: Option<BuildRuntime>,
    #[serde(default)]
    pub assessment: Option<AssessmentConfiguration>,
}

/// Shared Kubernetes execution settings for self-service Ayni Managed profiles.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRuntime {
    pub platform: String,
    #[serde(default = "runtime_account")]
    pub service_account: String,
    #[serde(default)]
    pub pull_secret: Option<String>,
    #[serde(default)]
    pub assessment: Option<AssessmentConfiguration>,
}

/// Only operator configuration can select the trusted collector and readiness policy.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentConfiguration {
    pub producer_image: String,
    #[serde(default)]
    pub publication_policy: PublicationPolicy,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PublicationPolicy {
    #[default]
    DraftOnly,
    ReadyAfterSuccessfulAssessment,
}

const fn interval() -> u64 {
    60
}
fn runtime_account() -> String {
    "runtime-coding".into()
}
const fn timeout() -> u64 {
    30
}
const fn concurrency() -> usize {
    4
}

impl RuntimeConfiguration {
    /// Loads and validates the optional operator-owned runtime configuration.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code when the configured file is unavailable,
    /// oversized, malformed, or contains unsupported execution settings.
    pub(crate) fn from_environment() -> Result<Option<Self>, &'static str> {
        let Ok(path) = env::var("ORCHESTRATOR_RUNTIME_CONFIG_PATH") else {
            return Ok(None);
        };
        let metadata = fs::metadata(&path).map_err(|_| "runtime_configuration_unavailable")?;
        if metadata.len() > 1_048_576 {
            return Err("runtime_configuration_oversized");
        }
        let config: Self = serde_json::from_slice(
            &fs::read(path).map_err(|_| "runtime_configuration_unavailable")?,
        )
        .map_err(|_| "runtime_configuration_invalid")?;
        config.validate()?;
        Ok(Some(config))
    }

    /// Bind deployment policy only to the connected repository's frozen namespace.
    #[must_use]
    pub(crate) fn managed_runtime(
        &self,
        owner: &str,
        repository: &str,
        lock_fingerprint: &str,
        oci_repository: &str,
    ) -> Option<ProfileRuntime> {
        let runtime = self
            .managed_runtime
            .as_ref()?
            .profile_runtime(owner, repository, lock_fingerprint)
            .ok()?;
        (image_repository(&runtime.image) == Some(oci_repository)).then_some(runtime)
    }

    /// Supplies only deployment fields needed to create execution resources.
    pub(crate) fn execution_configuration(&self) -> ExecutionConfiguration {
        ExecutionConfiguration {
            namespace: self.namespace.clone(),
            worker_image: self.worker_image.clone(),
            output_claim: self.output_claim.clone(),
            model_secret: self.model_secret.clone(),
            allow_emulation: self.allow_emulation,
        }
    }

    fn validate(&self) -> Result<(), &'static str> {
        self.execution_configuration().validate()?;
        if !(10..=3600).contains(&self.interval_seconds)
            || !(1..=120).contains(&self.request_timeout_seconds)
            || !(1..=16).contains(&self.concurrency)
            || self.profiles.len() > 128
        {
            return Err("runtime_configuration_invalid");
        }
        for (profile, runtime) in &self.profiles {
            validate_profile(profile, runtime)?;
        }
        if let Some(runtime) = &self.managed_runtime {
            runtime.validate()?;
        }
        Ok(())
    }
}

impl ProfileRuntime {
    /// Supplies only workload fields; assessment and publication remain application policy.
    pub(crate) fn execution_runtime(&self) -> WorkloadRuntime {
        WorkloadRuntime {
            image: self.image.clone(),
            platform: self.platform.clone(),
            service_account: self.service_account.clone(),
            pull_secret: self.pull_secret.clone(),
            build: self.build.clone(),
        }
    }
}

impl ManagedRuntime {
    /// Derives the only acceptable GHCR reference for a connected repository.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code when repository identity, lock identity, or
    /// operator execution settings are invalid.
    pub(crate) fn profile_runtime(
        &self,
        owner: &str,
        repository: &str,
        lock_fingerprint: &str,
    ) -> Result<ProfileRuntime, &'static str> {
        if !github_component(owner)
            || !github_component(repository)
            || lock_fingerprint.len() != 64
            || !lock_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("runtime_repository_identity_invalid");
        }
        let runtime = ProfileRuntime {
            image: format!(
                "ghcr.io/{}/{}-ayni:lock-{}",
                owner.to_lowercase(),
                repository.to_lowercase(),
                lock_fingerprint.to_lowercase()
            ),
            platform: self.platform.clone(),
            service_account: self.service_account.clone(),
            pull_secret: self.pull_secret.clone(),
            build: None,
            assessment: self.assessment.clone(),
        };
        validate_profile("managed", &runtime)?;
        Ok(runtime)
    }

    fn validate(&self) -> Result<(), &'static str> {
        self.profile_runtime("owner", "repository", &"a".repeat(64))
            .map(|_| ())
    }
}

fn validate_profile(profile: &str, runtime: &ProfileRuntime) -> Result<(), &'static str> {
    use ayni_orchestrator_runtime::runtime_contract::RuntimeImage;
    if runtime
        .assessment
        .as_ref()
        .is_some_and(|assessment| RuntimeImage::parse(&assessment.producer_image).is_err())
    {
        return Err("assessment_producer_invalid");
    }
    if profile.is_empty() || profile.len() > 256 {
        return Err("runtime_configuration_invalid");
    }
    runtime.execution_runtime().validate()
}

fn github_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
#[path = "runtime_configuration_tests.rs"]
mod tests;
