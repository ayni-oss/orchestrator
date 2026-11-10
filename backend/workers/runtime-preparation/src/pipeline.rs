//! Executable stages. The supervisor owns mounts, workload retirement and cleanup;
//! successful stages cannot advance until the exact pod has been removed.
use crate::{
    Failure,
    client::{Credential, WorkerClient},
    command,
    oci::{self, Layout},
    source, transport,
};
use ayni_core::{AyniPolicy, EnvironmentCertificateEnvelope, sha256_fingerprint};
use ayni_orchestrator_events::runtime_preparation::BuildMilestone;
use ayni_orchestrator_events::runtime_preparation::{
    Evidence, FrozenInputs, RuntimeProfile, StageClaim, StageReport,
};
use ayni_orchestrator_registry::Credentials;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// Supported public Ayni schema-1 report; it is consumed across isolated stages.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificationReport {
    pub schema_version: String,
    pub lock_fingerprint: String,
    pub platform: String,
    pub ayni_version: String,
    pub ayni_executable_digest: String,
    pub trusted_keys_fingerprint: String,
    pub certificate_digest: String,
    pub protected_content_root: String,
    pub key_id: String,
    pub layer_digest: String,
    pub diff_id: String,
    pub labels: BTreeMap<String, String>,
}

pub struct StageResult {
    pub outcome: &'static str,
    pub evidence: Evidence,
}
impl StageResult {
    const fn completed(evidence: Evidence) -> Self {
        Self {
            outcome: "completed",
            evidence,
        }
    }
}

/// # Errors
/// Fail with a closed stage reason. Credential-bearing stages cannot invoke
/// repository code, and builder stages never request a provider credential.
pub async fn execute(client: &WorkerClient, claim: &StageClaim) -> Result<StageResult, Failure> {
    if client.binding.operation_id != claim.operation.id
        || client.binding.stage != claim.attempt.stage
        || client.binding.generation != claim.attempt.generation
        || client.binding.pod_uid != claim.attempt.pod_uid
        || client.binding.command_digest != claim.attempt.command_digest
    {
        return Err(Failure::InvalidInput);
    }
    let profile: RuntimeProfile = serde_json::from_str(&claim.operation.frozen.profile_json)
        .map_err(|_| Failure::InvalidInput)?;
    let seconds = remaining_seconds(claim)?;
    match claim.attempt.stage.as_str() {
        "acquire_source" => acquire_source(client, claim, seconds).await,
        "discover" => discover(client, claim).await,
        "pull_existing" | "pull_final" => pull(client, claim, seconds).await,
        "build" => build(client, claim, &profile, seconds).await,
        "freeze_candidate" => freeze(claim, &profile, seconds).await,
        "certify" => certify(claim, &profile),
        "finalize" => finalize(claim, &profile, seconds).await,
        "publish" => publish(client, claim, seconds).await,
        // Independent content/managed probes have no worker capability; the
        // supervisor reads bounded launcher-owned results after pod completion.
        _ => Err(Failure::InvalidInput),
    }
}

fn remaining_seconds(claim: &StageClaim) -> Result<u64, Failure> {
    let now = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Failure::ExecutionDeadline)?
            .as_millis(),
    )
    .map_err(|_| Failure::ExecutionDeadline)?;
    let remaining = claim.attempt.deadline_unix_ms.saturating_sub(now) / 1000;
    if remaining == 0 {
        return Err(Failure::ExecutionDeadline);
    }
    Ok(remaining)
}
fn artifact(claim: &StageClaim, digest: &str) -> String {
    format!(
        "runtime://{}/{}/{}/{}",
        claim.operation.id,
        claim.attempt.stage,
        claim.attempt.generation,
        digest.trim_start_matches("sha256:")
    )
}
async fn registry_credential(client: &WorkerClient) -> Result<Credentials, Failure> {
    match client.credential().await? {
        Credential::Registry { username, password } => Ok(Credentials { username, password }),
        Credential::GithubInstallation { .. } => Err(Failure::InvalidInput),
    }
}
async fn acquire_source(
    client: &WorkerClient,
    claim: &StageClaim,
    seconds: u64,
) -> Result<StageResult, Failure> {
    let Credential::GithubInstallation { token } = client.credential().await? else {
        return Err(Failure::InvalidInput);
    };
    let frozen = &claim.operation.frozen;
    let digest = source::acquire(
        &frozen.repository_full_name,
        &frozen.source_commit,
        &frozen.source_tree,
        &token,
        Path::new("/outputs/checkout"),
        seconds,
    )
    .await?;
    verify_locked_source(Path::new("/outputs/checkout"), frozen)?;
    write_locked_source(Path::new("/outputs/locked"), frozen)?;
    Ok(StageResult::completed(Evidence {
        source_digest: digest.clone(),
        artifact_digest: digest.clone(),
        artifact_reference: artifact(claim, &digest),
        ..Evidence::default()
    }))
}

fn verify_locked_source(root: &Path, frozen: &FrozenInputs) -> Result<(), Failure> {
    for (path, expected) in [
        (".ayni.toml", frozen.policy_digest.as_str()),
        (".ayni.lock", frozen.lock_digest.as_str()),
    ] {
        if sha256_fingerprint(oci::read_beneath(root, path, 1024 * 1024)?) != expected {
            return Err(Failure::SourceIdentityMismatch);
        }
    }
    for file in &frozen.sources {
        if sha256_fingerprint(oci::read_beneath(root, &file.path, 512 * 1024)?) != file.digest {
            return Err(Failure::SourceIdentityMismatch);
        }
    }
    Ok(())
}
fn write_locked_source(root: &Path, frozen: &FrozenInputs) -> Result<(), Failure> {
    fs::create_dir(root).map_err(|_| Failure::ArtifactConflict)?;
    let mut files = BTreeMap::from([
        (".ayni.toml".to_owned(), frozen.policy.as_bytes()),
        (".ayni.lock".to_owned(), frozen.lock.as_bytes()),
    ]);
    for file in &frozen.sources {
        files.insert(file.path.clone(), &file.content);
    }
    for (relative, bytes) in files {
        if relative.is_empty()
            || Path::new(&relative)
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(Failure::SourcePathUnsupported);
        }
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or(Failure::SourcePathUnsupported)?)
            .map_err(|_| Failure::ExecutionFailed)?;
        fs::write(path, bytes).map_err(|_| Failure::ExecutionFailed)?;
    }
    Ok(())
}

async fn discover(client: &WorkerClient, claim: &StageClaim) -> Result<StageResult, Failure> {
    let op = &claim.operation;
    let credential = registry_credential(client).await?;
    if let Some(candidate) = retained_candidate(
        &op.frozen.reusable_image,
        &op.frozen.registry_destination,
        &op.evidence.expected_manifest_digest,
    )? && transport::discover_digest(
        &op.frozen.registry_destination,
        &candidate.evidence.image_digest,
        &op.platform,
        &credential,
    )
    .await?
    .is_some()
    {
        return Ok(candidate);
    }
    // A missing retained manifest falls through only after an authenticated 404.
    // Denial and outage already returned an unavailable failure.
    let tag = if op.evidence.expected_manifest_digest.is_empty() {
        &op.frozen.discovery_tag
    } else {
        &op.frozen.publication_tag
    };
    let candidate = transport::discover(
        &op.frozen.registry_destination,
        tag,
        &op.platform,
        &credential,
    )
    .await?;
    match candidate {
        None => Ok(StageResult {
            outcome: "missing",
            evidence: Evidence::default(),
        }),
        Some(candidate) => {
            if !op.evidence.expected_manifest_digest.is_empty()
                && op.evidence.expected_manifest_digest != candidate.manifest_digest
            {
                return Err(Failure::ArtifactConflict);
            }
            Ok(StageResult::completed(Evidence {
                candidate_image: candidate.image,
                image_digest: candidate.manifest_digest,
                ..Evidence::default()
            }))
        }
    }
}
fn retained_candidate(
    image: &str,
    destination: &str,
    publication_intent: &str,
) -> Result<Option<StageResult>, Failure> {
    if image.is_empty() || !publication_intent.is_empty() {
        return Ok(None);
    }
    let (repository, digest) = image.rsplit_once('@').ok_or(Failure::ArtifactInvalid)?;
    ayni_orchestrator_oci::ImageRepository::parse(repository)
        .map_err(|_| Failure::ArtifactInvalid)?;
    ayni_orchestrator_oci::Sha256Digest::parse(digest).map_err(|_| Failure::ArtifactInvalid)?;
    if repository != destination {
        return Err(Failure::ArtifactConflict);
    }
    // This selects a previously admitted immutable candidate. Actual registry
    // pull, current trust/content checks, managed checks and cleanup still follow.
    Ok(Some(StageResult::completed(Evidence {
        candidate_image: image.into(),
        image_digest: digest.into(),
        ..Evidence::default()
    })))
}
async fn pull(
    client: &WorkerClient,
    claim: &StageClaim,
    seconds: u64,
) -> Result<StageResult, Failure> {
    let credential = registry_credential(client).await?;
    let op = &claim.operation;
    let hash = op
        .evidence
        .candidate_image
        .rsplit_once('@')
        .ok_or(Failure::InvalidInput)?
        .1;
    let identity = transport::pull(
        &op.frozen.registry_destination,
        hash,
        &op.platform,
        &credential,
        Path::new("/outputs/layout"),
        claim.attempt.stage == "pull_final",
        transport::PullLimits {
            maximum_bytes: u64::from(
                serde_json::from_str::<RuntimeProfile>(&op.frozen.profile_json)
                    .map_err(|_| Failure::InvalidInput)?
                    .max_storage_mib,
            ) * 1024
                * 1024,
            seconds,
        },
    )
    .await?;
    Ok(StageResult::completed(Evidence {
        image_digest: hash.into(),
        artifact_digest: identity.manifest_digest.clone(),
        artifact_reference: artifact(claim, &identity.manifest_digest),
        ..Evidence::default()
    }))
}

async fn build(
    client: &WorkerClient,
    claim: &StageClaim,
    profile: &RuntimeProfile,
    seconds: u64,
) -> Result<StageResult, Failure> {
    let root = Path::new("/scratch/source");
    client.build_progress(BuildMilestone::CopySource).await;
    copy_source(Path::new("/inputs/source/checkout"), root)?;
    if source::snapshot_digest(root)? != claim.operation.evidence.source_digest {
        return Err(Failure::SourceIdentityMismatch);
    }
    client.build_progress(BuildMilestone::VerifyInputs).await;
    verify_locked_source(root, &claim.operation.frozen)?;
    let image = format!("localhost/ayni-runtime:{}", claim.operation.id);
    let environment = BTreeMap::from([
        ("HOME".into(), "/home/podman".into()),
        ("XDG_RUNTIME_DIR".into(), "/scratch/run".into()),
        (
            "CONTAINERS_STORAGE_CONF".into(),
            "/runtime-config/storage.conf".into(),
        ),
        (
            "CONTAINERS_CONF".into(),
            "/runtime-config/containers.conf".into(),
        ),
        ("BUILDAH_LAYERS".into(), "false".into()),
        ("AYNI_OCI_RUNTIME".into(), "podman".into()),
    ]);
    fs::create_dir_all("/scratch/run").map_err(|_| Failure::ExecutionFailed)?;
    client.build_progress(BuildMilestone::BuildImage).await;
    command::checked(
        "/usr/local/bin/ayni",
        &[
            "env".into(),
            "build".into(),
            "--repo-root".into(),
            root.to_string_lossy().into_owned(),
            "--executor-image".into(),
            profile.executor_image.clone(),
            "--tag".into(),
            image.clone(),
        ],
        root,
        &environment,
        seconds,
    )
    .await?;
    client.build_progress(BuildMilestone::ExportImage).await;
    command::checked(
        "/usr/bin/podman",
        &[
            "save".into(),
            "--format".into(),
            "oci-dir".into(),
            "--output".into(),
            "/outputs/layout".into(),
            image,
        ],
        root,
        &environment,
        seconds,
    )
    .await?;
    client.build_progress(BuildMilestone::InspectImage).await;
    normalize_reference(Path::new("/outputs/layout"))?;
    let identity = Layout::inspect_with_limit(
        Path::new("/outputs/layout"),
        &profile.platform,
        u64::from(profile.max_storage_mib) * 1024 * 1024,
    )?
    .identity;
    client.build_progress(BuildMilestone::ImageReady).await;
    Ok(StageResult::completed(Evidence {
        artifact_digest: identity.manifest_digest.clone(),
        artifact_reference: artifact(claim, &identity.manifest_digest),
        ..Evidence::default()
    }))
}

fn normalize_reference(layout: &Path) -> Result<(), Failure> {
    let index = layout.join("index.json");
    let mut value: serde_json::Value = serde_json::from_slice(&read(&index, 4 * 1024 * 1024)?)
        .map_err(|_| Failure::ArtifactInvalid)?;
    if value["manifests"]
        .as_array()
        .is_none_or(|manifests| manifests.len() != 1)
    {
        return Err(Failure::ArtifactInvalid);
    }
    value["manifests"][0]["annotations"]["org.opencontainers.image.ref.name"] = "runtime".into();
    fs::write(
        index,
        serde_json::to_vec(&value).map_err(|_| Failure::ArtifactInvalid)?,
    )
    .map_err(|_| Failure::ExecutionFailed)
}
async fn freeze(
    claim: &StageClaim,
    profile: &RuntimeProfile,
    seconds: u64,
) -> Result<StageResult, Failure> {
    let layout = Layout::inspect_with_limit(
        Path::new("/inputs/candidate/layout"),
        &profile.platform,
        u64::from(profile.max_storage_mib) * 1024 * 1024,
    )?;
    if layout.identity.manifest_digest != claim.operation.evidence.artifact_digest {
        return Err(Failure::ArtifactConflict);
    }
    crate::extraction::preflight(
        Path::new("/inputs/candidate/layout"),
        &layout.identity,
        u64::from(profile.max_storage_mib) * 1024 * 1024,
    )?;
    layout.freeze(Path::new("/outputs/layout"))?;
    oci::unpack(
        Path::new("/outputs/layout"),
        Path::new("/outputs/bundle"),
        seconds,
    )
    .await?;
    let root = Path::new("/outputs/bundle/rootfs");
    let executable = oci::read_beneath(root, "usr/local/bin/ayni", 256 * 1024 * 1024)?;
    if sha256_fingerprint(executable) != profile.trusted_cli_sha256 {
        return Err(Failure::ArtifactInvalid);
    }
    let manifest = oci::read_beneath(
        root,
        "etc/ayni/protected-content.manifest",
        ayni_environment::MAX_PROTECTED_MANIFEST_BYTES,
    )?;
    let text = std::str::from_utf8(&manifest).map_err(|_| Failure::ArtifactInvalid)?;
    ayni_environment::parse_protected_manifest(text).map_err(|_| Failure::ArtifactInvalid)?;
    // This freezes the selected unsigned manifest BEFORE giving any stage signing
    // authority. The official certifier then independently inventories actual bytes
    // and must match this externally retained root; it cannot invent a new root.
    let expected_root = sha256_fingerprint(&manifest);
    let unsigned: EnvironmentCertificateEnvelope = serde_json::from_slice(&oci::read_beneath(
        root,
        "etc/ayni/runtime.json",
        ayni_environment::MAX_CERTIFICATE_BYTES,
    )?)
    .map_err(|_| Failure::ArtifactInvalid)?;
    if unsigned.certificate.lock_fingerprint != claim.operation.frozen.lock_fingerprint
        || unsigned.certificate.platform != profile.platform
        || unsigned.certificate.protected_content_root != expected_root
    {
        return Err(Failure::ArtifactInvalid);
    }
    Ok(StageResult::completed(Evidence {
        artifact_digest: layout.identity.manifest_digest.clone(),
        artifact_reference: artifact(claim, &layout.identity.manifest_digest),
        unsigned_manifest_digest: layout.identity.manifest_digest.clone(),
        protected_content_root: expected_root,
        ..Evidence::default()
    }))
}

fn certify(claim: &StageClaim, profile: &RuntimeProfile) -> Result<StageResult, Failure> {
    let frozen = &claim.operation.frozen;
    let root = Path::new("/inputs/source/locked");
    verify_locked_source(root, frozen)?;
    let policy = AyniPolicy::parse(&frozen.policy).map_err(|_| Failure::CertificationFailed)?;
    let trust = policy
        .environment_certificate_trust_policy()
        .map_err(|_| Failure::CertificationFailed)?;
    let lock = ayni_environment::read_lock(root).map_err(|_| Failure::CertificationFailed)?;
    let mut seed = read_seed()?;
    if seed.last() == Some(&b'\n') {
        seed.pop();
    }
    let seed = std::str::from_utf8(&seed).map_err(|_| Failure::CertificationFailed)?;
    let expectations = ayni_environment::staged::CertificationExpectations {
        platform: &profile.platform,
        ayni_executable_digest: &profile.trusted_cli_sha256,
        protected_content_root: &claim.operation.evidence.protected_content_root,
    };
    let layer = ayni_environment::staged::certify_filesystem(
        Path::new("/inputs/frozen/bundle/rootfs"),
        &lock,
        &trust,
        &expectations,
        &frozen.signer_key_id,
        seed,
    )
    .map_err(|_| Failure::CertificationFailed)?;
    layer
        .write(Path::new("/outputs/layer.tar"))
        .map_err(|_| Failure::CertificationFailed)?;
    fs::write(
        "/outputs/report.json",
        serde_json::to_vec(&layer.report).map_err(|_| Failure::CertificationFailed)?,
    )
    .map_err(|_| Failure::CertificationFailed)?;
    Ok(StageResult::completed(Evidence {
        certificate_digest: layer.report.certificate_digest,
        certification_layer_digest: layer.report.layer_digest.clone(),
        artifact_digest: layer.report.layer_digest.clone(),
        artifact_reference: artifact(claim, &layer.report.layer_digest),
        ..Evidence::default()
    }))
}
async fn finalize(
    claim: &StageClaim,
    profile: &RuntimeProfile,
    seconds: u64,
) -> Result<StageResult, Failure> {
    let report: CertificationReport = serde_json::from_slice(&oci::read_beneath(
        Path::new("/inputs/certification"),
        "report.json",
        64 * 1024,
    )?)
    .map_err(|_| Failure::ArtifactInvalid)?;
    if report.platform != profile.platform
        || report.layer_digest != claim.operation.evidence.certification_layer_digest
        || report.schema_version != "1"
        || report.lock_fingerprint != claim.operation.frozen.lock_fingerprint
        || report.key_id != claim.operation.frozen.signer_key_id
        || report.protected_content_root != claim.operation.evidence.protected_content_root
        || report.certificate_digest != claim.operation.evidence.certificate_digest
        || report.ayni_executable_digest != profile.trusted_cli_sha256
    {
        return Err(Failure::ArtifactConflict);
    }
    let layout = Layout::inspect(Path::new("/inputs/frozen/layout"), &profile.platform)?;
    if layout.identity.manifest_digest != claim.operation.evidence.unsigned_manifest_digest {
        return Err(Failure::ArtifactConflict);
    }
    layout.freeze(Path::new("/outputs/layout"))?;
    let identity = oci::append_certification(
        Path::new("/outputs/layout"),
        Path::new("/inputs/certification/layer.tar"),
        &report,
        seconds,
    )
    .await?;
    Ok(StageResult::completed(Evidence {
        expected_manifest_digest: identity.manifest_digest.clone(),
        artifact_digest: identity.manifest_digest.clone(),
        artifact_reference: artifact(claim, &identity.manifest_digest),
        ..Evidence::default()
    }))
}
async fn publish(
    client: &WorkerClient,
    claim: &StageClaim,
    seconds: u64,
) -> Result<StageResult, Failure> {
    let op = &claim.operation;
    let identity = Layout::inspect(Path::new("/inputs/final/layout"), &op.platform)?.identity;
    client
        .prepare_publication(&identity.manifest_digest)
        .await?;
    let credential = registry_credential(client).await?;
    transport::publish(
        &op.frozen.registry_destination,
        &op.frozen.publication_tag,
        Path::new("/inputs/final/layout"),
        &identity.manifest_digest,
        &op.platform,
        &credential,
        seconds,
    )
    .await?;
    Ok(StageResult::completed(Evidence {
        image_digest: identity.manifest_digest,
        ..Evidence::default()
    }))
}

/// Bounded source copy to writable build/compatibility scratch. Inputs are a
/// verified source-acquisition output; links or special files are unsupported.
/// # Errors
/// Reject changed source, occupied destination and IO errors.
pub fn copy_source(source: &Path, destination: &Path) -> Result<(), Failure> {
    source::snapshot_digest(source)?;
    copy_source_inner(source, destination)
}
fn copy_source_inner(source: &Path, destination: &Path) -> Result<(), Failure> {
    fs::create_dir(destination).map_err(|_| Failure::ArtifactConflict)?;
    for entry in fs::read_dir(source).map_err(|_| Failure::SourceUnavailable)? {
        let entry = entry.map_err(|_| Failure::SourceUnavailable)?;
        let input = entry.path();
        let output = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&input).map_err(|_| Failure::SourceUnavailable)?;
        if metadata.is_dir() {
            copy_source_inner(&input, &output)?;
        } else if metadata.is_file() {
            fs::copy(&input, output).map_err(|_| Failure::ExecutionFailed)?;
        } else {
            return Err(Failure::SourcePathUnsupported);
        }
    }
    Ok(())
}
fn read_seed() -> Result<Vec<u8>, Failure> {
    // Kubernetes Secret projections use symlinks. Only this fixed administrator-
    // owned, UID-fenced Secret mount may follow them; it is absent from candidates.
    let file = fs::File::open("/runtime-signer/seed").map_err(|_| Failure::CertificationFailed)?;
    let metadata = file.metadata().map_err(|_| Failure::CertificationFailed)?;
    if !metadata.is_file() || metadata.len() > 65 {
        return Err(Failure::CertificationFailed);
    }
    let mut bytes = Vec::new();
    file.take(66)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::CertificationFailed)?;
    if bytes.len() > 65 {
        return Err(Failure::CertificationFailed);
    }
    Ok(bytes)
}
fn read(path: &Path, maximum: u64) -> Result<Vec<u8>, Failure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Failure::ArtifactInvalid)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    let mut result = Vec::new();
    fs::File::open(path)
        .map_err(|_| Failure::ArtifactInvalid)?
        .take(maximum + 1)
        .read_to_end(&mut result)
        .map_err(|_| Failure::ArtifactInvalid)?;
    if result.len() as u64 > maximum {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    Ok(result)
}

/// Retain a closed failure without raw child/provider diagnostics. Any publication
/// error after an admitted intent remains uncertain until exact readback.
#[must_use]
pub fn failure_report(client: &WorkerClient, reason: Failure) -> StageReport {
    StageReport {
        binding: client.binding.clone(),
        outcome: if client.binding.stage == "publish" {
            "publication_uncertain"
        } else if matches!(
            reason,
            Failure::RegistryAccessDenied
                | Failure::RegistryUnavailable
                | Failure::SourceUnavailable
                | Failure::SourceTreeUnavailable
                | Failure::SourceFetchUnavailable
                | Failure::SourceAccessDenied
                | Failure::SourceRateLimited
        ) {
            "unavailable"
        } else {
            "failed"
        }
        .into(),
        safe_reason: reason.reason().into(),
        evidence: Evidence::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
    #[test]
    fn retained_candidates_are_exact_and_never_replace_publication_reconciliation() -> TestResult {
        let digest = oci::digest(b"manifest");
        let image = format!("ghcr.io/fixture/runtime@{digest}");
        let selected = retained_candidate(&image, "ghcr.io/fixture/runtime", "")?
            .ok_or("retained candidate absent")?;
        assert_eq!(selected.evidence.candidate_image, image);
        assert!(!selected.evidence.content_verified);
        assert!(!selected.evidence.managed_compatible);
        assert!(retained_candidate(&image, "ghcr.io/fixture/runtime", &digest)?.is_none());
        assert!(retained_candidate(&image, "ghcr.io/other/runtime", "").is_err());
        assert!(
            retained_candidate(
                "ghcr.io/fixture/runtime:latest",
                "ghcr.io/fixture/runtime",
                ""
            )
            .is_err()
        );
        Ok(())
    }
}
