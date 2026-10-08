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
    /// Maximum model/tool turns available to one coding attempt.
    #[serde(default = "coding_turns")]
    pub coding_turns: u16,
    #[serde(default = "quality_repair_limit")]
    pub quality_repair_limit: u16,
    #[serde(default = "concurrency")]
    pub concurrency: usize,
    /// Local Docker Desktop only; production keeps platform node selection.
    #[serde(default)]
    pub allow_emulation: bool,
    /// Infrastructure boundary selected by the deployment administrator.
    #[serde(default = "external_engine_mode")]
    pub engine_mode: String,
    #[serde(default)]
    pub builder_defaults: Option<BuilderDefaults>,
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
pub struct BuilderDefaults {
    #[serde(default)]
    pub builder_image: String,
    pub engine_image: String,
    pub provisioning_base: String,
    #[serde(default)]
    pub test_registry_id: String,
    #[serde(default)]
    pub test_certificate_id: String,
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
}

/// Shared Kubernetes execution settings for self-service Ayni Managed profiles.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRuntime {
    /// Operator-controlled registry endpoint and namespace prefix. Repository
    /// source never chooses where a runtime image is published.
    #[serde(default = "managed_registry_prefix")]
    pub registry_prefix: String,
    pub platform: String,
    #[serde(default = "runtime_account")]
    pub service_account: String,
    #[serde(default)]
    pub pull_secret: Option<String>,
    /// Build-only authority remains in the internal Job's `ServiceAccount`.
    #[serde(default)]
    pub build: Option<BuildRuntime>,
}

fn external_engine_mode() -> String {
    "external-isolated".into()
}

const fn interval() -> u64 {
    60
}
fn runtime_account() -> String {
    "runtime-coding".into()
}
fn managed_registry_prefix() -> String {
    "ghcr.io".into()
}
const fn timeout() -> u64 {
    30
}
const fn coding_turns() -> u16 {
    24
}
const fn quality_repair_limit() -> u16 {
    3
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
        _oci_repository: &str,
    ) -> Option<ProfileRuntime> {
        let runtime = self
            .managed_runtime
            .as_ref()?
            .profile_runtime(owner, repository, lock_fingerprint)
            .ok()?;
        Some(runtime)
    }

    /// Supplies only deployment fields needed to create execution resources.
    pub(crate) fn execution_configuration(&self) -> ExecutionConfiguration {
        ExecutionConfiguration {
            namespace: self.namespace.clone(),
            worker_image: self.worker_image.clone(),
            output_claim: self.output_claim.clone(),
            model_secret: self.model_secret.clone(),
            allow_emulation: self.allow_emulation,
            engine_mode: self.engine_mode.clone(),
        }
    }

    fn validate(&self) -> Result<(), &'static str> {
        self.execution_configuration().validate()?;
        if !(10..=3600).contains(&self.interval_seconds)
            || !(1..=120).contains(&self.request_timeout_seconds)
            || !(1..=128).contains(&self.coding_turns)
            || self.quality_repair_limit > 10
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
        for build in self
            .profiles
            .values()
            .filter_map(|r| r.build.as_ref())
            .chain(self.managed_runtime.as_ref().and_then(|r| r.build.as_ref()))
        {
            let mode = build
                .engine
                .as_ref()
                .map_or("external-isolated", |e| e.security_mode.as_str());
            if mode != self.engine_mode {
                return Err("engine_deployment_mode_mismatch");
            }
        }
        Ok(())
    }
}

impl ProfileRuntime {
    /// Supplies the workload fields for the worker-owned execution.
    pub(crate) fn execution_runtime(&self) -> WorkloadRuntime {
        WorkloadRuntime {
            image: self.image.clone(),
            platform: self.platform.clone(),
            service_account: self.service_account.clone(),
            pull_secret: self.pull_secret.clone(),
            build: self.build.clone(),
        }
    }

    /// Rebinds a managed runtime's mutable lookup tag to the lock observed from
    /// the repository. The repository portion remains operator-configured, while
    /// each lock fingerprint receives its own image identity.
    pub(crate) fn for_lock_fingerprint(&self, fingerprint: &str) -> Result<Self, &'static str> {
        let repository = image_repository(&self.image).ok_or("runtime_image_invalid")?;
        let mut runtime = self.clone();
        runtime.image = managed_image_reference(repository, &self.platform, fingerprint)?;
        runtime.execution_runtime().validate()?;
        Ok(runtime)
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
        if !github_component(owner) || !github_component(repository) {
            return Err("runtime_repository_identity_invalid");
        }
        let mut runtime = ProfileRuntime {
            image: format!(
                "{}/{}/{}",
                self.registry_prefix.trim_end_matches('/'),
                owner.to_lowercase(),
                repository.to_lowercase(),
            ),
            platform: self.platform.clone(),
            service_account: self.service_account.clone(),
            pull_secret: self.pull_secret.clone(),
            build: self.build.clone(),
        };
        runtime.image = managed_image_reference(&runtime.image, &self.platform, lock_fingerprint)?;
        validate_profile("managed", &runtime)?;
        Ok(runtime)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.registry_prefix.is_empty()
            || self.registry_prefix.len() > 256
            || !self.registry_prefix.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'/' | b'-' | b'_')
            })
        {
            return Err("runtime_registry_prefix_invalid");
        }
        self.profile_runtime("owner", "repository", &"a".repeat(64))
            .map(|_| ())
    }
}

fn validate_profile(profile: &str, runtime: &ProfileRuntime) -> Result<(), &'static str> {
    if profile.is_empty() || profile.len() > 256 {
        return Err("runtime_configuration_invalid");
    }
    runtime.execution_runtime().validate()
}

/// Connection-owned GHCR destination. Preserve foreign-owner routing while
/// avoiding a repeated namespace when the source repository has the same owner.
///
/// # Errors
///
/// Returns a bounded error code for an invalid namespace or repository identity.
pub fn connection_image_repository(
    namespace: &str,
    owner: &str,
    repository: &str,
) -> Result<String, &'static str> {
    if !crate::registry_probe::valid_namespace(namespace)
        || !github_component(owner)
        || !github_component(repository)
    {
        return Err("runtime_repository_identity_invalid");
    }
    let namespace = namespace.to_ascii_lowercase();
    let owner = owner.to_ascii_lowercase();
    let repository = repository.to_ascii_lowercase();
    let path = if namespace == owner {
        repository
    } else {
        format!("{owner}/{repository}")
    };
    let destination = format!("ghcr.io/{namespace}/{path}");
    managed_image_reference(&destination, "linux/amd64", &"a".repeat(64))?;
    Ok(destination)
}

fn github_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests;

/// Canonical discovery reference; the tag is never admission authority.
pub fn managed_image_reference(
    repository: &str,
    platform: &str,
    fingerprint: &str,
) -> Result<String, &'static str> {
    let fingerprint = fingerprint.strip_prefix("sha256:").unwrap_or(fingerprint);
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("runtime_lock_fingerprint_invalid");
    }
    let architecture = match platform {
        "linux/amd64" => "amd64",
        "linux/arm64" => "arm64",
        _ => return Err("runtime_platform_invalid"),
    };
    let path = repository.split_once('/').ok_or("runtime_image_invalid")?.1;
    let image = format!(
        "{repository}:ayni-{architecture}-{}",
        fingerprint.to_ascii_lowercase()
    );
    if path.len() > 255 || image.len() > 512 || image_repository(&image).is_none() {
        return Err("runtime_image_invalid");
    }
    Ok(image)
}
