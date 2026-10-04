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
    /// Docker Desktop can emulate a foreign image architecture for local-only
    /// development. Production keeps the platform node selector enabled.
    #[serde(default)]
    pub allow_emulation: bool,
    #[serde(default)]
    pub managed_repositories: BTreeMap<String, ProfileRuntime>,
    #[serde(default)]
    pub managed_defaults: ManagedDefaults,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileRuntime>,
}

/// Deployment-wide execution policy for repositories enrolled through onboarding.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedDefaults {
    pub platform: String,
    pub service_account: String,
    pub pull_secret: Option<String>,
    pub assessment: Option<AssessmentConfiguration>,
}

impl Default for ManagedDefaults {
    fn default() -> Self {
        Self {
            platform: "linux/amd64".into(),
            service_account: runtime_account(),
            pull_secret: None,
            assessment: None,
        }
    }
}

impl ManagedDefaults {
    fn runtime(&self, image: String) -> ProfileRuntime {
        ProfileRuntime {
            image,
            platform: self.platform.clone(),
            service_account: self.service_account.clone(),
            pull_secret: self.pull_secret.clone(),
            assessment: self.assessment.clone(),
            build: None,
        }
    }
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
    300
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

    /// Infer only the onboarding-owned GHCR namespace. Explicit repository policy
    /// overrides deployment defaults; an explicit profile is handled by the caller.
    pub(crate) fn managed_runtime(
        &self,
        github: &ayni_orchestrator_api::v1::GithubRepository,
        managed: &ayni_orchestrator_api::v1::AyniManagedProfileRevision,
    ) -> Option<ProfileRuntime> {
        let repository = format!("{}/{}", github.owner, github.name);
        let expected = format!("ghcr.io/{}-ayni", repository.to_lowercase());
        if managed.oci_repository != expected
            || !ayni_orchestrator_runtime::assessment::digest(&format!(
                "sha256:{}",
                managed.lock_fingerprint
            ))
        {
            return None;
        }
        let image = format!("{expected}:lock-{}", managed.lock_fingerprint);
        let mut runtime = self
            .managed_repositories
            .get(&repository)
            .cloned()
            .unwrap_or_else(|| self.managed_defaults.runtime(image));
        runtime.image = runtime.image.replace("{lock}", &managed.lock_fingerprint);
        (image_repository(&runtime.image) == Some(expected.as_str())).then_some(runtime)
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
            || self.profiles.len() + self.managed_repositories.len() > 128
        {
            return Err("runtime_configuration_invalid");
        }
        validate_profile(
            "managed-defaults",
            &self.managed_defaults.runtime(format!(
                "ghcr.io/example/repository-ayni:lock-{}",
                "a".repeat(64)
            )),
        )?;
        for (profile, runtime) in &self.profiles {
            validate_profile(profile, runtime)?;
        }

        for (repository, runtime) in &self.managed_repositories {
            let parts: Vec<_> = repository.split('/').collect();
            if parts.len() != 2
                || parts.iter().any(|p| {
                    p.is_empty()
                        || !p
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                })
                || runtime.build.is_some()
            {
                return Err("runtime_repository_configuration_invalid");
            }
            let expected = format!("ghcr.io/{}-ayni:lock-{{lock}}", repository.to_lowercase());
            if runtime.image != expected {
                return Err("runtime_repository_configuration_invalid");
            }
            let mut resolved = runtime.clone();
            resolved.image = runtime.image.replace("{lock}", &"a".repeat(64));
            validate_profile(repository, &resolved)?;
        }
        Ok(())
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
    fn configuration() -> RuntimeConfiguration {
        RuntimeConfiguration {
            namespace: "test".into(),
            output_claim: "output".into(),
            model_secret: "model".into(),
            worker_image: format!("registry.test/worker@sha256:{}", "a".repeat(64)),
            interval_seconds: 60,
            request_timeout_seconds: 30,
            concurrency: 4,
            allow_emulation: false,
            profiles: BTreeMap::new(),
            managed_repositories: BTreeMap::new(),
            managed_defaults: ManagedDefaults::default(),
        }
    }

    #[test]
    fn rejects_unbounded_scheduling_and_credential_values() {
        let mut configuration = configuration();
        assert!(configuration.validate().is_ok());
        configuration.concurrency = 0;
        assert!(configuration.validate().is_err());
        assert!(!name("token:value"));
        assert!(!name("../secret"));
    }
    #[test]
    fn onboarding_derives_runtime_without_repository_configuration() {
        let mut configuration = configuration();
        configuration.managed_defaults.pull_secret = Some("runtime-pull".into());
        let github = ayni_orchestrator_api::v1::GithubRepository {
            owner: "Example".into(),
            name: "Sandbox".into(),
            ..Default::default()
        };
        let mut managed = ayni_orchestrator_api::v1::AyniManagedProfileRevision {
            oci_repository: "ghcr.io/example/sandbox-ayni".into(),
            lock_fingerprint: "a".repeat(64),
            ..Default::default()
        };
        let Some(runtime) = configuration.managed_runtime(&github, &managed) else {
            panic!("onboarded repository must resolve without an operator repository entry");
        };
        assert_eq!(
            runtime.image,
            format!("ghcr.io/example/sandbox-ayni:lock-{}", "a".repeat(64))
        );
        assert_eq!(runtime.platform, "linux/amd64");
        assert_eq!(runtime.service_account, "runtime-coding");
        assert_eq!(runtime.pull_secret.as_deref(), Some("runtime-pull"));
        assert!(runtime.build.is_none());
        managed.lock_fingerprint = "b".repeat(64);
        assert!(
            configuration
                .managed_runtime(&github, &managed)
                .is_some_and(|r| r.image.ends_with(&"b".repeat(64)))
        );
        managed.oci_repository = "ghcr.io/another/repository-ayni".into();
        assert!(configuration.managed_runtime(&github, &managed).is_none());
        managed.oci_repository = "ghcr.io/example/sandbox-ayni".into();
        managed.lock_fingerprint = "invalid".into();
        assert!(configuration.managed_runtime(&github, &managed).is_none());
    }

    #[test]
    fn repository_override_takes_precedence_over_managed_defaults() {
        let mut configuration = configuration();
        let github = ayni_orchestrator_api::v1::GithubRepository {
            owner: "example".into(),
            name: "sandbox".into(),
            ..Default::default()
        };
        let managed = ayni_orchestrator_api::v1::AyniManagedProfileRevision {
            oci_repository: "ghcr.io/example/sandbox-ayni".into(),
            lock_fingerprint: "a".repeat(64),
            ..Default::default()
        };
        let mut runtime = configuration
            .managed_defaults
            .runtime("ghcr.io/example/sandbox-ayni:lock-{lock}".into());
        runtime.service_account = "custom-account".into();
        runtime.platform = "linux/arm64".into();
        configuration
            .managed_repositories
            .insert("example/sandbox".into(), runtime);
        assert!(configuration.validate().is_ok());
        let Some(runtime) = configuration.managed_runtime(&github, &managed) else {
            panic!("repository override must resolve");
        };
        assert_eq!(runtime.service_account, "custom-account");
        assert_eq!(runtime.platform, "linux/arm64");
        assert!(runtime.image.ends_with(&managed.lock_fingerprint));
    }

    #[test]
    fn managed_defaults_validate_platform_and_secret_references() {
        let mut configuration = configuration();
        configuration.managed_defaults.platform = "unknown".into();
        assert!(configuration.validate().is_err());
        configuration.managed_defaults.platform = "linux/amd64".into();
        configuration.managed_defaults.pull_secret = Some("../secret".into());
        assert!(configuration.validate().is_err());
        configuration.managed_defaults.pull_secret = Some("runtime-pull".into());
        assert!(configuration.validate().is_ok());
    }
}
