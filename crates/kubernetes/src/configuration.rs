//! Execution resource settings supplied by the application boundary.
//!
//! Scheduling, profile selection, repository discovery, and publication policy
//! belong to the control plane. Credentials here are Kubernetes Secret names.

use ayni_orchestrator_runtime::runtime_contract::{RuntimeImage, RuntimePlatform};
use serde::{Deserialize, Serialize};

/// Deployment resources required by the Kubernetes execution adapter.
#[derive(Clone)]
pub struct ExecutionConfiguration {
    pub namespace: String,
    pub worker_image: String,
    pub output_claim: String,
    pub model_secret: String,
    /// Local Docker Desktop only; production keeps platform node selection.
    pub allow_emulation: bool,
}

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
    pub image: String,
    #[serde(default = "builder_account")]
    pub service_account: String,
    pub registry_secret: String,
    pub signing_secret: String,
    pub engine_secret: String,
    pub engine_host: String,
}

fn builder_account() -> String {
    "runtime-builder".into()
}

impl ExecutionConfiguration {
    /// Validates deployment resource names and the trusted worker image identity.
    ///
    /// # Errors
    /// Returns a bounded error code for malformed names or an unpinned worker image.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !name(&self.namespace)
            || !name(&self.output_claim)
            || !name(&self.model_secret)
            || RuntimeImage::parse(&self.worker_image).is_err()
        {
            return Err("runtime_configuration_invalid");
        }
        Ok(())
    }
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
            && (RuntimeImage::parse(&build.image).is_err()
                || !name(&build.service_account)
                || !name(&build.registry_secret)
                || !name(&build.signing_secret)
                || !name(&build.engine_secret)
                || self.image.contains('@')
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
}

/// Returns the repository portion of a validated OCI image reference.
#[must_use]
pub fn image_repository(image: &str) -> Option<&str> {
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

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
