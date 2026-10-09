//! Application workload admission and isolation settings.
use ayni_orchestrator_runtime::runtime_contract::{
    ImageRepository, ImageTag, RuntimeImage, RuntimePlatform,
};
use serde::{Deserialize, Serialize};

/// Runtime settings used to construct one isolated workload.
#[derive(Clone)]
pub struct WorkloadRuntime {
    pub image: String,
    pub platform: String,
    pub service_account: String,
    pub pull_secret: Option<String>,
    pub build: Option<BuildRuntime>,
}

/// Dedicated build workload settings; no credential contents enter this value.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRuntime {
    #[serde(default)]
    pub configuration_revision: u64,
    /// Populated from repository resolution; omitted by infrastructure configuration.
    #[serde(default)]
    pub image: String,
    #[serde(default = "builder_account")]
    pub service_account: String,
    #[serde(default)]
    pub registry_secret: String,
    #[serde(default)]
    pub signing_secret: String,
    #[serde(default)]
    pub engine_secret: String,
    #[serde(default)]
    pub engine_host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineRuntime>,
    /// Legacy external-engine operator attestation: this engine is isolated from Kubernetes nodes.
    #[serde(default)]
    pub isolated_engine: bool,
}

/// Per-attempt Kubernetes Docker engine. Production and development are explicit.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineRuntime {
    pub image: String,
    pub security_mode: String,
}

impl EngineRuntime {
    /// Validate the pinned engine and explicit isolation mode.
    /// # Errors
    /// Rejects mutable images, unknown modes.
    pub fn validate(&self) -> Result<(), &'static str> {
        if RuntimeImage::parse(&self.image).is_err()
            || !matches!(
                self.security_mode.as_str(),
                "dedicated-builder" | "local-development"
            )
        {
            return Err("runtime_engine_configuration_invalid");
        }
        Ok(())
    }
}

fn builder_account() -> String {
    "runtime-builder".into()
}

impl WorkloadRuntime {
    /// Validates workload identities and optional build-only authority.
    ///
    /// # Errors
    /// Returns a bounded error code for unsafe workload settings, Secret names,
    /// builder identities, or a build engine without the supported TLS endpoint.
    pub fn validate(&self) -> Result<(), &'static str> {
        if RuntimePlatform::parse(&self.platform).is_err()
            || !name(&self.service_account)
            || self
                .pull_secret
                .as_deref()
                .is_some_and(|secret| !name(secret))
            || self.image.is_empty()
            || self.image.len() > 512
            || image_repository(&self.image).is_none()
            || !self.image.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'@')
            })
        {
            return Err("runtime_configuration_invalid");
        }
        if let Some(build) = &self.build
            && ((build.engine.is_none() && !build.isolated_engine)
                || (!build.image.is_empty() && RuntimeImage::parse(&build.image).is_err())
                || !name(&build.service_account)
                || (!build.registry_secret.is_empty() && !name(&build.registry_secret))
                || (!build.signing_secret.is_empty() && !name(&build.signing_secret))
                || (build.engine.is_none() && !name(&build.engine_secret))
                || self.image.contains('@')
                || (build.engine.is_none()
                    && (!build.engine_host.starts_with("tcp://")
                        || build.engine_host.len() <= 11
                        || !build.engine_host.ends_with(":2376")
                        || !build.engine_host[6..build.engine_host.len() - 5]
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')))))
        {
            return Err("runtime_build_configuration_invalid");
        }
        if let Some(engine) = self.build.as_ref().and_then(|b| b.engine.as_ref()) {
            engine.validate()?;
        }
        Ok(())
    }
}

/// Returns the repository portion of a validated OCI image reference.
#[must_use]
pub fn image_repository(image: &str) -> Option<&str> {
    if image.contains('@') {
        RuntimeImage::parse(image).ok()?;
        let repository = image.split_once('@')?.0;
        let (_, path) = repository.split_once('/')?;
        return (path.len() <= 255).then_some(repository);
    }
    let (repository, tag) = image.rsplit_once(':')?;
    ImageTag::parse(tag).ok()?;
    ImageRepository::parse(repository).ok()?;
    let (_, path) = repository.split_once('/')?;
    if path.len() > 255 {
        return None;
    }
    Some(repository)
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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

impl ProfileRuntime {
    /// Supplies the workload fields for the worker-owned execution.
    #[must_use]
    pub fn execution_runtime(&self) -> WorkloadRuntime {
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
    ///
    /// # Errors
    /// Rejects invalid image, platform, fingerprint or workload settings.
    pub fn for_lock_fingerprint(&self, fingerprint: &str) -> Result<Self, &'static str> {
        let repository = image_repository(&self.image).ok_or("runtime_image_invalid")?;
        let mut runtime = self.clone();
        runtime.image = managed_image_reference(repository, &self.platform, fingerprint)?;
        runtime.execution_runtime().validate()?;
        Ok(runtime)
    }
}

/// Canonical discovery reference; the tag is never admission authority.
///
/// # Errors
/// Rejects unsupported platforms, malformed fingerprints and image repositories.
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

fn runtime_account() -> String {
    "runtime-coding".into()
}
