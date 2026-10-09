//! Application workload admission and isolation settings.
use ayni_orchestrator_runtime::runtime_contract::{RuntimeImage, RuntimePlatform};
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
    if tag.is_empty()
        || tag.len() > 128
        || tag.starts_with(['-', '.'])
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        || repository
            .split_once('/')
            .is_none_or(|(_, path)| path.len() > 255)
    {
        return None;
    }
    RuntimeImage::parse(&format!("{repository}@sha256:{}", "a".repeat(64))).ok()?;
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
