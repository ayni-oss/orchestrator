//! Stage-scoped registry IO. Authentication files live only in private temporary
//! storage, never in arguments, artifacts, reports or repository-readable volumes.
use crate::{Failure, command, oci::Layout};
use ayni_orchestrator_oci::{ImageRepository, ImageTag, Sha256Digest};
use ayni_orchestrator_registry::{
    Candidate, Credentials, Discovery, DiscoveryError, GhcrClient, Platform,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{collections::BTreeMap, io::Write as _, path::Path};

fn platform(value: &str) -> Result<Platform, Failure> {
    match value {
        "linux/arm64" => Ok(Platform::LinuxArm64),
        "linux/amd64" => Ok(Platform::LinuxAmd64),
        _ => Err(Failure::PlatformMismatch),
    }
}
const fn discovery_error(error: DiscoveryError) -> Failure {
    match error {
        DiscoveryError::AccessDenied => Failure::RegistryAccessDenied,
        DiscoveryError::Unavailable | DiscoveryError::RateLimited => Failure::RegistryUnavailable,
        DiscoveryError::PlatformUnavailable => Failure::PlatformMismatch,
        DiscoveryError::InvalidInput
        | DiscoveryError::ResponseInvalid
        | DiscoveryError::ResponseOversized => Failure::ArtifactInvalid,
    }
}
/// Actual registry discovery with the operation's frozen pull identity.
/// # Errors
/// Access, rate limits, transport and malformed responses never become absence.
pub async fn discover(
    repository: &str,
    tag: &str,
    selected: &str,
    credential: &Credentials,
) -> Result<Option<Candidate>, Failure> {
    let client = GhcrClient::new().map_err(discovery_error)?;
    match client
        .discover(repository, tag, platform(selected)?, credential)
        .await
        .map_err(discovery_error)?
    {
        Discovery::Missing => Ok(None),
        Discovery::Candidate(candidate) => Ok(Some(candidate)),
    }
}

/// Check actual immutable registry availability using the frozen pull account.
/// # Errors
/// Authentication and outage never become missing or permission to build.
pub async fn discover_digest(
    repository: &str,
    digest: &str,
    selected: &str,
    credential: &Credentials,
) -> Result<Option<Candidate>, Failure> {
    let client = GhcrClient::new().map_err(discovery_error)?;
    match client
        .discover_digest(repository, digest, platform(selected)?, credential)
        .await
        .map_err(discovery_error)?
    {
        Discovery::Missing => Ok(None),
        Discovery::Candidate(candidate) => Ok(Some(candidate)),
    }
}

fn auth_file(credential: &Credentials) -> Result<tempfile::NamedTempFile, Failure> {
    let mut file = tempfile::NamedTempFile::new().map_err(|_| Failure::ExecutionFailed)?;
    let value = serde_json::json!({"auths":{"ghcr.io":{"auth":STANDARD.encode(format!("{}:{}", credential.username, credential.password))}}});
    file.write_all(
        serde_json::to_string(&value)
            .map_err(|_| Failure::ExecutionFailed)?
            .as_bytes(),
    )
    .map_err(|_| Failure::ExecutionFailed)?;
    file.flush().map_err(|_| Failure::ExecutionFailed)?;
    Ok(file)
}
fn repository(value: &str) -> Result<(), Failure> {
    ImageRepository::parse(value).map_err(|_| Failure::InvalidInput)?;
    if !value.starts_with("ghcr.io/") {
        return Err(Failure::InvalidInput);
    }
    Ok(())
}

pub struct PullLimits {
    pub maximum_bytes: u64,
    pub seconds: u64,
}

/// Pull one immutable manifest using only a pull account.
///
/// Existing Docker media may be normalized to OCI; the original registry digest remains separate evidence.
/// Final publication readback must preserve the exact published OCI manifest digest.
/// # Errors
/// Retain a closed actual-execution failure; never retry a different reference.
pub async fn pull(
    repository_name: &str,
    digest: &str,
    selected: &str,
    credential: &Credentials,
    output: &Path,
    preserve_digest: bool,
    limits: PullLimits,
) -> Result<crate::oci::Identity, Failure> {
    let PullLimits {
        maximum_bytes: maximum,
        seconds,
    } = limits;
    repository(repository_name)?;
    Sha256Digest::parse(digest).map_err(|_| Failure::InvalidInput)?;
    let architecture = platform(selected)?
        .as_str()
        .strip_prefix("linux/")
        .ok_or(Failure::PlatformMismatch)?;
    let auth = auth_file(credential)?;
    manifest_preflight(repository_name, digest, &auth, maximum, seconds).await?;
    let mut args = vec![
        "--override-os".into(),
        "linux".into(),
        "--override-arch".into(),
        architecture.into(),
        "copy".into(),
        "--authfile".into(),
        auth.path().to_string_lossy().into_owned(),
    ];
    if preserve_digest {
        args.push("--preserve-digests".into());
    } else {
        args.extend(["--format".into(), "oci".into()]);
    }
    args.extend([
        format!("docker://{repository_name}@{digest}"),
        format!("oci:{}:runtime", output.display()),
    ]);
    let output_status = command::run(
        "/usr/bin/skopeo",
        &args,
        Path::new("/"),
        &BTreeMap::new(),
        seconds,
    )
    .await?;
    if !output_status.success {
        return Err(output_status.registry_failure());
    }
    let identity = Layout::inspect_with_limit(output, selected, maximum)?.identity;
    if preserve_digest && identity.manifest_digest != digest {
        return Err(Failure::ArtifactInvalid);
    }
    Ok(identity)
}

async fn manifest_preflight(
    repository: &str,
    digest: &str,
    auth: &tempfile::NamedTempFile,
    maximum: u64,
    seconds: u64,
) -> Result<(), Failure> {
    let args = [
        "inspect".into(),
        "--raw".into(),
        "--authfile".into(),
        auth.path().to_string_lossy().into_owned(),
        format!("docker://{repository}@{digest}"),
    ];
    let response = command::run(
        "/usr/bin/skopeo",
        &args,
        Path::new("/"),
        &BTreeMap::new(),
        seconds,
    )
    .await?;
    if !response.success {
        return Err(response.registry_failure());
    }
    declared_transfer_bytes(&response.stdout, digest, maximum)?;
    Ok(())
}
fn declared_transfer_bytes(bytes: &[u8], expected: &str, maximum: u64) -> Result<u64, Failure> {
    if crate::oci::digest(bytes) != expected {
        return Err(Failure::ArtifactInvalid);
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| Failure::ArtifactInvalid)?;
    if manifest["schemaVersion"] != 2
        || !matches!(
            manifest["mediaType"].as_str(),
            Some(
                "application/vnd.oci.image.manifest.v1+json"
                    | "application/vnd.docker.distribution.manifest.v2+json"
            )
        )
    {
        return Err(Failure::ArtifactInvalid);
    }
    let layers = manifest["layers"]
        .as_array()
        .ok_or(Failure::ArtifactInvalid)?;
    if layers.is_empty() || layers.len() > 256 {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    let mut total = u64::try_from(bytes.len()).map_err(|_| Failure::ArtifactBoundsExceeded)?;
    for descriptor in std::iter::once(&manifest["config"]).chain(layers) {
        if descriptor.get("urls").is_some() || descriptor.get("data").is_some() {
            return Err(Failure::ArtifactInvalid);
        }
        Sha256Digest::parse(
            descriptor["digest"]
                .as_str()
                .ok_or(Failure::ArtifactInvalid)?,
        )
        .map_err(|_| Failure::ArtifactInvalid)?;
        let size = descriptor["size"]
            .as_u64()
            .ok_or(Failure::ArtifactInvalid)?;
        total = total
            .checked_add(size)
            .ok_or(Failure::ArtifactBoundsExceeded)?;
        if size == 0 || total > maximum {
            return Err(Failure::ArtifactBoundsExceeded);
        }
    }
    Ok(total)
}

/// Publish the already finalized layout. The caller MUST persist its intended
/// manifest digest with authority before calling this function.
///
/// Any interruption
/// requires exact registry readback, even if the client observed a failure.
/// # Errors
/// An uncertain side effect never authorizes rebuilding or changing the destination.
pub async fn publish(
    repository_name: &str,
    tag: &str,
    layout: &Path,
    expected_digest: &str,
    selected: &str,
    credential: &Credentials,
    seconds: u64,
) -> Result<(), Failure> {
    repository(repository_name)?;
    ImageTag::parse(tag).map_err(|_| Failure::InvalidInput)?;
    let identity = Layout::inspect(layout, selected)?.identity;
    if identity.manifest_digest != expected_digest {
        return Err(Failure::ArtifactConflict);
    }
    let auth = auth_file(credential)?;
    let args = [
        "copy".into(),
        "--authfile".into(),
        auth.path().to_string_lossy().into_owned(),
        "--preserve-digests".into(),
        format!("oci:{}:runtime", layout.display()),
        format!("docker://{repository_name}:{tag}"),
    ];
    let result = command::run(
        "/usr/bin/skopeo",
        &args,
        Path::new("/"),
        &BTreeMap::new(),
        seconds,
    )
    .await;
    // Even denied/failed uploads may follow earlier accepted blobs or manifest
    // storage. Keep their specific safe reason as diagnostic, but side-effect
    // state must remain publication_uncertain until the pull stage reads back.
    match result {
        Ok(status) if status.success => Ok(()),
        Ok(status) => Err(status.registry_failure()),
        Err(_) => Err(Failure::PublicationUncertain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_manifest_transfer_budget_precedes_blob_download() {
        let manifest=serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"digest":format!("sha256:{}","a".repeat(64)),"size":1},"layers":[{"digest":format!("sha256:{}","b".repeat(64)),"size":100_000}]})).unwrap_or_default();
        let hash = crate::oci::digest(&manifest);
        assert!(declared_transfer_bytes(&manifest, &hash, 200_000).is_ok());
        assert!(matches!(
            declared_transfer_bytes(&manifest, &hash, 50_000),
            Err(Failure::ArtifactBoundsExceeded)
        ));
        assert!(matches!(
            declared_transfer_bytes(&manifest, &format!("sha256:{}", "0".repeat(64)), 200_000),
            Err(Failure::ArtifactInvalid)
        ));
    }
}
