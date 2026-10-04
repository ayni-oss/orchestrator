//! Operator-owned Kubernetes runtime configuration; all credentials are references.
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRuntime {
    pub image: String,
    #[serde(default = "builder_account")]
    pub service_account: String,
    pub registry_secret: String,
    pub signing_secret: String,
    pub engine_secret: String,
    pub engine_host: String,
}

const fn interval() -> u64 {
    60
}
fn runtime_account() -> String {
    "runtime-coding".into()
}
fn builder_account() -> String {
    "runtime-builder".into()
}
const fn timeout() -> u64 {
    30
}
const fn concurrency() -> usize {
    4
}

impl RuntimeConfiguration {
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

    fn validate(&self) -> Result<(), &'static str> {
        use ayni_orchestrator_runtime::runtime_contract::RuntimeImage;
        if !name(&self.namespace)
            || !name(&self.output_claim)
            || !name(&self.model_secret)
            || RuntimeImage::parse(&self.worker_image).is_err()
            || !(10..=3600).contains(&self.interval_seconds)
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

impl ManagedRuntime {
    /// Derives the only acceptable GHCR reference for a connected repository.
    pub fn profile_runtime(
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
    use ayni_orchestrator_runtime::runtime_contract::{RuntimeImage, RuntimePlatform};
    if runtime
        .assessment
        .as_ref()
        .is_some_and(|assessment| RuntimeImage::parse(&assessment.producer_image).is_err())
    {
        return Err("assessment_producer_invalid");
    }
    if profile.is_empty()
        || profile.len() > 256
        || RuntimePlatform::parse(&runtime.platform).is_err()
        || !name(&runtime.service_account)
        || runtime
            .pull_secret
            .as_deref()
            .is_some_and(|secret| !name(secret))
        || runtime.image.is_empty()
        || runtime.image.len() > 512
        || image_repository(&runtime.image).is_none()
        || !runtime.image.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'@')
        })
    {
        return Err("runtime_configuration_invalid");
    }
    if let Some(build) = &runtime.build
        && (RuntimeImage::parse(&build.image).is_err()
            || !name(&build.service_account)
            || !name(&build.registry_secret)
            || !name(&build.signing_secret)
            || !name(&build.engine_secret)
            || runtime.image.contains('@')
            || !build.engine_host.starts_with("tcp://")
            || build.engine_host.len() <= 11
            || !build.engine_host.ends_with(":2376")
            || !build.engine_host[6..build.engine_host.len() - 5]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')))
    {
        return Err("runtime_build_configuration_invalid");
    }
    Ok(())
}

fn github_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn image_repository(image: &str) -> Option<&str> {
    use ayni_orchestrator_runtime::runtime_contract::RuntimeImage;
    if image.contains('@') {
        RuntimeImage::parse(image).ok()?;
        return image.split_once('@').map(|(repository, _)| repository);
    }
    let (repository, tag) = image.rsplit_once(':')?;
    if tag.is_empty() || tag.len() > 128 || tag.contains('/') || tag.starts_with(['-', '.']) {
        return None;
    }
    RuntimeImage::parse(&format!("{repository}@sha256:{}", "a".repeat(64))).ok()?;
    Some(repository)
}

pub fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_scheduling_and_credential_values() {
        let mut configuration = RuntimeConfiguration {
            namespace: "test".into(),
            output_claim: "output".into(),
            model_secret: "model".into(),
            worker_image: format!("registry.test/worker@sha256:{}", "a".repeat(64)),
            interval_seconds: 60,
            request_timeout_seconds: 30,
            concurrency: 4,
            profiles: BTreeMap::new(),
            managed_runtime: None,
        };
        assert!(configuration.validate().is_ok());
        configuration.concurrency = 0;
        assert!(configuration.validate().is_err());
        assert!(!name("token:value"));
        assert!(!name("../secret"));
    }

    #[test]
    fn derives_managed_image_from_connected_repository_and_lock() -> Result<(), &'static str> {
        let runtime = ManagedRuntime {
            platform: "linux/amd64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: Some("environment-pull".into()),
            assessment: None,
        };
        let profile = runtime.profile_runtime("Ayni-Oss", "sandbox", &"aB".repeat(32))?;
        assert_eq!(
            profile.image,
            format!("ghcr.io/ayni-oss/sandbox-ayni:lock-{}", "ab".repeat(32))
        );
        assert!(
            runtime
                .profile_runtime("../owner", "sandbox", &"a".repeat(64))
                .is_err()
        );
        Ok(())
    }
}
