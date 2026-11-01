//! Immutable OCI transport validation. OCI tools own extraction and layer/config
//! updates; this module verifies their exact descriptor bytes before handoff.
use crate::{Failure, command};
use ayni_orchestrator_oci::Sha256Digest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read as _,
    path::{Component, Path, PathBuf},
};

const JSON_BYTES: u64 = 4 * 1024 * 1024;
const BLOB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_LAYERS: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Descriptor {
    pub digest: String,
    pub size: u64,
    #[serde(rename = "mediaType")]
    pub media_type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub manifest_digest: String,
    pub config_digest: String,
    pub platform: String,
    pub layers: Vec<Descriptor>,
}
pub struct Layout {
    root: PathBuf,
    pub identity: Identity,
    config: Descriptor,
}

impl Layout {
    /// Inspect a single-platform, single-reference layout and every referenced blob.
    /// There are no URLs, decompression, candidate commands or external OCI references.
    /// # Errors
    /// Reject missing/changed/oversized objects, unsupported layout shapes or platform drift.
    pub fn inspect(root: &Path, expected_platform: &str) -> Result<Self, Failure> {
        Self::inspect_with_limit(root, expected_platform, TOTAL_BYTES)
    }
    /// # Errors
    /// Enforce the operation profile's compressed object bound as well as byte identity.
    pub fn inspect_with_limit(
        root: &Path,
        expected_platform: &str,
        maximum: u64,
    ) -> Result<Self, Failure> {
        directory(root)?;
        directory(&root.join("blobs"))?;
        directory(&root.join("blobs/sha256"))?;
        let format: Value =
            serde_json::from_slice(&read_regular(&root.join("oci-layout"), JSON_BYTES)?)
                .map_err(|_| Failure::ArtifactInvalid)?;
        if format["imageLayoutVersion"] != "1.0.0" {
            return Err(Failure::ArtifactInvalid);
        }
        let index: Value =
            serde_json::from_slice(&read_regular(&root.join("index.json"), JSON_BYTES)?)
                .map_err(|_| Failure::ArtifactInvalid)?;
        let manifests = index["manifests"]
            .as_array()
            .ok_or(Failure::ArtifactInvalid)?;
        if index["schemaVersion"] != 2
            || manifests.len() != 1
            || manifests[0]["annotations"]["org.opencontainers.image.ref.name"] != "runtime"
        {
            return Err(Failure::ArtifactInvalid);
        }
        let descriptor = decode_descriptor(&manifests[0], JSON_BYTES)?;
        if descriptor.media_type != "application/vnd.oci.image.manifest.v1+json" {
            return Err(Failure::ArtifactInvalid);
        }
        let manifest = json_blob(root, &descriptor)?;
        if manifest["schemaVersion"] != 2 {
            return Err(Failure::ArtifactInvalid);
        }
        let config = decode_descriptor(&manifest["config"], JSON_BYTES)?;
        if config.media_type != "application/vnd.oci.image.config.v1+json" {
            return Err(Failure::ArtifactInvalid);
        }
        let configuration = json_blob(root, &config)?;
        let platform = format!(
            "{}/{}",
            configuration["os"]
                .as_str()
                .ok_or(Failure::ArtifactInvalid)?,
            configuration["architecture"]
                .as_str()
                .ok_or(Failure::ArtifactInvalid)?
        );
        if platform != expected_platform {
            return Err(Failure::PlatformMismatch);
        }
        let layers = inspect_layers(root, &manifest, &configuration, maximum)?;
        Ok(Self {
            root: root.into(),
            identity: Identity {
                manifest_digest: descriptor.digest,
                config_digest: config.digest.clone(),
                platform,
                layers,
            },
            config,
        })
    }

    /// Copy only verified OCI objects to a fresh stage-owned store. A retired
    /// builder cannot reach this destination or its independently retained identity.
    /// # Errors
    /// Reject conflicting destination or any object changed since inspection.
    pub fn freeze(&self, destination: &Path) -> Result<(), Failure> {
        fs::create_dir(destination).map_err(|_| Failure::ArtifactConflict)?;
        fs::create_dir_all(destination.join("blobs/sha256"))
            .map_err(|_| Failure::ExecutionFailed)?;
        for name in ["oci-layout", "index.json"] {
            let bytes = read_regular(&self.root.join(name), JSON_BYTES)?;
            fs::write(destination.join(name), bytes).map_err(|_| Failure::ExecutionFailed)?;
        }
        let mut copied = BTreeSet::new();
        for descriptor in std::iter::once(Descriptor {
            digest: self.identity.manifest_digest.clone(),
            size: fs::metadata(blob_path(&self.root, &self.identity.manifest_digest)?)
                .map_err(|_| Failure::ArtifactInvalid)?
                .len(),
            media_type: "application/vnd.oci.image.manifest.v1+json".into(),
        })
        .chain(std::iter::once(self.config.clone()))
        .chain(self.identity.layers.iter().cloned())
        {
            if !copied.insert(descriptor.digest.clone()) {
                continue;
            }
            let original = blob_path(&self.root, &descriptor.digest)?;
            let target = blob_path(destination, &descriptor.digest)?;
            fs::copy(original, &target).map_err(|_| Failure::ExecutionFailed)?;
            verify_blob(destination, &descriptor)?;
        }
        let observed = Self::inspect(destination, &self.identity.platform)?;
        if observed.identity.manifest_digest != self.identity.manifest_digest {
            return Err(Failure::ArtifactConflict);
        }
        Ok(())
    }
}

fn inspect_layers(
    root: &Path,
    manifest: &Value,
    config: &Value,
    maximum: u64,
) -> Result<Vec<Descriptor>, Failure> {
    let layers = manifest["layers"]
        .as_array()
        .ok_or(Failure::ArtifactInvalid)?;
    let diff_ids = config["rootfs"]["diff_ids"]
        .as_array()
        .ok_or(Failure::ArtifactInvalid)?;
    if layers.is_empty()
        || layers.len() > MAX_LAYERS
        || layers.len() != diff_ids.len()
        || config["rootfs"]["type"] != "layers"
    {
        return Err(Failure::ArtifactInvalid);
    }
    let mut total = 0_u64;
    let mut result = Vec::new();
    for (layer, diff_id) in layers.iter().zip(diff_ids) {
        Sha256Digest::parse(diff_id.as_str().ok_or(Failure::ArtifactInvalid)?)
            .map_err(|_| Failure::ArtifactInvalid)?;
        let descriptor = decode_descriptor(layer, BLOB_BYTES)?;
        if !matches!(
            descriptor.media_type.as_str(),
            "application/vnd.oci.image.layer.v1.tar"
                | "application/vnd.oci.image.layer.v1.tar+gzip"
                | "application/vnd.oci.image.layer.v1.tar+zstd"
        ) {
            return Err(Failure::ArtifactInvalid);
        }
        total = total
            .checked_add(descriptor.size)
            .ok_or(Failure::ArtifactBoundsExceeded)?;
        if total > maximum.min(TOTAL_BYTES) {
            return Err(Failure::ArtifactBoundsExceeded);
        }
        verify_blob(root, &descriptor)?;
        result.push(descriptor);
    }
    Ok(result)
}
fn decode_descriptor(value: &Value, maximum: u64) -> Result<Descriptor, Failure> {
    if value.get("urls").is_some() || value.get("data").is_some() {
        return Err(Failure::ArtifactInvalid);
    }
    let descriptor: Descriptor =
        serde_json::from_value(value.clone()).map_err(|_| Failure::ArtifactInvalid)?;
    Sha256Digest::parse(&descriptor.digest).map_err(|_| Failure::ArtifactInvalid)?;
    if descriptor.size == 0 || descriptor.size > maximum {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    Ok(descriptor)
}
fn json_blob(root: &Path, descriptor: &Descriptor) -> Result<Value, Failure> {
    let bytes = read_regular(&blob_path(root, &descriptor.digest)?, JSON_BYTES)?;
    if bytes.len() as u64 != descriptor.size || digest(&bytes) != descriptor.digest {
        return Err(Failure::ArtifactInvalid);
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::ArtifactInvalid)
}
fn verify_blob(root: &Path, descriptor: &Descriptor) -> Result<(), Failure> {
    let path = blob_path(root, &descriptor.digest)?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::ArtifactInvalid)?;
    if !metadata.is_file() || metadata.len() != descriptor.size {
        return Err(Failure::ArtifactInvalid);
    }
    let file = fs::File::open(path).map_err(|_| Failure::ArtifactInvalid)?;
    let mut file = file.take(descriptor.size + 1);
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 65_536].into_boxed_slice();
    let mut count = 0_u64;
    loop {
        let bytes = file
            .read(&mut buffer)
            .map_err(|_| Failure::ArtifactInvalid)?;
        if bytes == 0 {
            break;
        }
        count += bytes as u64;
        if count > descriptor.size {
            return Err(Failure::ArtifactInvalid);
        }
        hash.update(&buffer[..bytes]);
    }
    if count != descriptor.size || format!("sha256:{:x}", hash.finalize()) != descriptor.digest {
        return Err(Failure::ArtifactInvalid);
    }
    Ok(())
}
fn blob_path(root: &Path, digest: &str) -> Result<PathBuf, Failure> {
    let digest = Sha256Digest::parse(digest).map_err(|_| Failure::ArtifactInvalid)?;
    Ok(root.join("blobs/sha256").join(digest.hex()))
}
fn directory(path: &Path) -> Result<(), Failure> {
    if !fs::symlink_metadata(path)
        .map_err(|_| Failure::ArtifactInvalid)?
        .is_dir()
    {
        return Err(Failure::ArtifactInvalid);
    }
    Ok(())
}
/// Read an immutable stage input relative to one retained root without following
/// any candidate-controlled ancestor. The supervisor must retire prior writers.
pub(crate) fn read_beneath(root: &Path, relative: &str, maximum: u64) -> Result<Vec<u8>, Failure> {
    directory(root)?;
    let mut components = Path::new(relative).components().peekable();
    let mut path = root.to_path_buf();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(Failure::ArtifactInvalid);
        };
        path.push(name);
        if components.peek().is_none() {
            return read_regular(&path, maximum);
        }
        directory(&path)?;
    }
    Err(Failure::ArtifactInvalid)
}
fn read_regular(path: &Path, maximum: u64) -> Result<Vec<u8>, Failure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Failure::ArtifactInvalid)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| Failure::ArtifactInvalid)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::ArtifactInvalid)?;
    if bytes.len() as u64 > maximum {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    Ok(bytes)
}
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Extract using the pinned OCI implementation, never the candidate's entrypoint.
/// Kubernetes storage quotas and deadlines bound extraction and retained storage.
/// # Errors
/// Reject tool failures; extraction never grants runtime trust.
pub async fn unpack(layout: &Path, bundle: &Path, seconds: u64) -> Result<(), Failure> {
    command::checked(
        "/usr/bin/umoci",
        &[
            "unpack".into(),
            "--rootless".into(),
            "--image".into(),
            format!("{}:runtime", layout.display()),
            bundle.to_string_lossy().into_owned(),
        ],
        Path::new("/"),
        &BTreeMap::new(),
        seconds,
    )
    .await?;
    Ok(())
}

/// Append the exact official metadata layer and labels without evaluating image
/// configuration, ONBUILD triggers or repository commands.
/// # Errors
/// Reject conflicting layer identity, malformed labels or OCI tool failure.
pub async fn append_certification(
    layout: &Path,
    layer: &Path,
    report: &crate::pipeline::CertificationReport,
    seconds: u64,
) -> Result<Identity, Failure> {
    if digest(&read_beneath(
        layer.parent().ok_or(Failure::ArtifactInvalid)?,
        layer
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(Failure::ArtifactInvalid)?,
        128 * 1024 * 1024,
    )?) != report.layer_digest
        || report.diff_id != report.layer_digest
    {
        return Err(Failure::ArtifactInvalid);
    }
    let image = format!("{}:runtime", layout.display());
    command::checked(
        "/usr/bin/umoci",
        &[
            "raw".into(),
            "add-layer".into(),
            "--image".into(),
            image.clone(),
            "--history.created".into(),
            "1970-01-01T00:00:00Z".into(),
            "--history.created_by".into(),
            "ayni-staged-certification".into(),
            "--compress".into(),
            "none".into(),
            layer.to_string_lossy().into_owned(),
        ],
        Path::new("/"),
        &BTreeMap::new(),
        seconds,
    )
    .await?;
    let mut arguments = vec![
        "config".into(),
        "--image".into(),
        image,
        "--no-history".into(),
    ];
    for (key, value) in &report.labels {
        if !key.starts_with("dev.ayni.environment.") || value.contains(['\n', '\0']) {
            return Err(Failure::ArtifactInvalid);
        }
        arguments.extend(["--config.label".into(), format!("{key}={value}")]);
    }
    command::checked(
        "/usr/bin/umoci",
        &arguments,
        Path::new("/"),
        &BTreeMap::new(),
        seconds,
    )
    .await?;
    Ok(Layout::inspect(layout, &report.platform)?.identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn fixture() -> Result<(tempfile::TempDir, String), Box<dyn std::error::Error + Send + Sync>> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("blobs/sha256"))?;
        let layer = vec![0_u8; 1024];
        let layer_digest = digest(&layer);
        fs::write(blob_path(root.path(), &layer_digest)?, &layer)?;
        let config = serde_json::to_vec(
            &serde_json::json!({"os":"linux","architecture":"arm64","rootfs":{"type":"layers","diff_ids":[layer_digest]}}),
        )?;
        let config_digest = digest(&config);
        fs::write(blob_path(root.path(), &config_digest)?, &config)?;
        let manifest = serde_json::to_vec(
            &serde_json::json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":config_digest,"size":config.len()},"layers":[{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":layer_digest,"size":layer.len()}]}),
        )?;
        let manifest_digest = digest(&manifest);
        fs::write(blob_path(root.path(), &manifest_digest)?, &manifest)?;
        fs::write(
            root.path().join("index.json"),
            serde_json::to_vec(
                &serde_json::json!({"schemaVersion":2,"manifests":[{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":manifest_digest,"size":manifest.len(),"annotations":{"org.opencontainers.image.ref.name":"runtime"}}]}),
            )?,
        )?;
        fs::write(
            root.path().join("oci-layout"),
            b"{\"imageLayoutVersion\":\"1.0.0\"}",
        )?;
        Ok((root, layer_digest))
    }
    #[test]
    fn actual_blob_bytes_platform_and_operation_bounds_are_checked() -> TestResult {
        let (root, layer) = fixture()?;
        assert!(Layout::inspect(root.path(), "linux/arm64").is_ok());
        assert!(matches!(
            Layout::inspect(root.path(), "linux/amd64"),
            Err(Failure::PlatformMismatch)
        ));
        assert!(matches!(
            Layout::inspect_with_limit(root.path(), "linux/arm64", 512),
            Err(Failure::ArtifactBoundsExceeded)
        ));
        fs::write(blob_path(root.path(), &layer)?, vec![1_u8; 1024])?;
        assert!(Layout::inspect(root.path(), "linux/arm64").is_err());
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn symlink_blob_and_symlink_layout_directory_are_rejected() -> TestResult {
        let (root, layer) = fixture()?;
        let path = blob_path(root.path(), &layer)?;
        let outside = tempfile::NamedTempFile::new()?;
        fs::write(outside.path(), vec![0_u8; 1024])?;
        fs::remove_file(&path)?;
        std::os::unix::fs::symlink(outside.path(), path)?;
        assert!(Layout::inspect(root.path(), "linux/arm64").is_err());
        let parent = tempfile::tempdir()?;
        let link = parent.path().join("layout");
        std::os::unix::fs::symlink(root.path(), &link)?;
        assert!(Layout::inspect(&link, "linux/arm64").is_err());
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn anchored_reads_reject_candidate_symlink_ancestors_before_opening_files() -> TestResult {
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        fs::create_dir(outside.path().join("bin"))?;
        fs::write(outside.path().join("bin/ayni"), b"outside candidate")?;
        std::os::unix::fs::symlink(outside.path(), root.path().join("usr"))?;
        assert!(read_beneath(root.path(), "usr/bin/ayni", 1024).is_err());
        assert!(read_beneath(root.path(), "../outside", 1024).is_err());
        fs::create_dir(root.path().join("etc"))?;
        fs::write(root.path().join("etc/runtime.json"), b"retained input")?;
        assert_eq!(
            read_beneath(root.path(), "etc/runtime.json", 1024)?,
            b"retained input"
        );
        Ok(())
    }
}
