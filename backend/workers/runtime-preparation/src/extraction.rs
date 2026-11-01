//! Bound OCI extraction before writing retained filesystems.
//!
//! Storage
//! requests alone are not quotas: bound expanded tar bytes, entries and file sizes
//! in trusted streaming readers before permitting extraction to a retained volume.
use crate::{Failure, oci::Identity};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Component, Path},
};

const MAX_ENTRIES: u64 = 250_000;
const MAX_FILE: u64 = 1024 * 1024 * 1024;

struct Budget<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for Budget<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let capacity =
            usize::try_from(self.remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        if capacity == 0 {
            let mut probe = [0_u8; 1];
            return if self.inner.read(&mut probe)? == 0 {
                Ok(0)
            } else {
                Err(io::Error::other("expanded OCI budget exceeded"))
            };
        }
        let count = self.inner.read(&mut buffer[..capacity])?;
        self.remaining -= count as u64;
        Ok(count)
    }
}

/// # Errors
/// Bound expanded layer data against the operation's declared allowance. Count
/// all layers conservatively, including files later deleted. Filesystem metadata
/// and allocation policy still require the deployment's outer storage controls.
pub fn preflight(layout: &Path, identity: &Identity, maximum: u64) -> Result<(), Failure> {
    let compressed = identity
        .layers
        .iter()
        .try_fold(0_u64, |sum, layer| sum.checked_add(layer.size))
        .ok_or(Failure::ArtifactBoundsExceeded)?;
    // Reserve metadata/config and filesystem-directory overhead. The kubelet also
    // enforces the outer pod's declared ephemeral limit and wall-clock deadline.
    let mut expanded = maximum
        .checked_sub(compressed)
        .and_then(|left| left.checked_sub(64 * 1024 * 1024))
        .ok_or(Failure::ArtifactBoundsExceeded)?;
    let mut files = MAX_ENTRIES;
    let mut directories = BTreeSet::new();
    for layer in &identity.layers {
        let hash = ayni_orchestrator_oci::Sha256Digest::parse(&layer.digest)
            .map_err(|_| Failure::ArtifactInvalid)?;
        let file = fs::File::open(layout.join("blobs/sha256").join(hash.hex()))
            .map_err(|_| Failure::ArtifactInvalid)?;
        let reader: Box<dyn Read> = match layer.media_type.as_str() {
            "application/vnd.oci.image.layer.v1.tar" => Box::new(file),
            "application/vnd.oci.image.layer.v1.tar+gzip" => {
                Box::new(flate2::read::MultiGzDecoder::new(file))
            }
            "application/vnd.oci.image.layer.v1.tar+zstd" => {
                let mut decoder =
                    zstd::stream::read::Decoder::new(file).map_err(|_| Failure::ArtifactInvalid)?;
                decoder
                    .window_log_max(27)
                    .map_err(|_| Failure::ArtifactBoundsExceeded)?;
                Box::new(decoder)
            }
            _ => return Err(Failure::ArtifactInvalid),
        };
        let before = expanded;
        let mut archive = tar::Archive::new(Budget {
            inner: reader,
            remaining: expanded,
        });
        for entry in archive
            .entries()
            .map_err(|_| Failure::ArtifactInvalid)?
            .raw(true)
        {
            let mut entry = entry.map_err(|_| Failure::ArtifactBoundsExceeded)?;
            files = files
                .checked_sub(1)
                .ok_or(Failure::ArtifactBoundsExceeded)?;
            let path = entry
                .path()
                .map_err(|_| Failure::ArtifactInvalid)?
                .into_owned();
            validate_entry(&entry, &path)?;
            validate_extensions(&mut entry)?;
            for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
                if directories.insert(ancestor.to_path_buf()) {
                    files = files
                        .checked_sub(1)
                        .ok_or(Failure::ArtifactBoundsExceeded)?;
                }
            }
        }
        let mut reader = archive.into_inner();
        io::copy(&mut reader, &mut io::sink()).map_err(|_| Failure::ArtifactBoundsExceeded)?;
        expanded = reader.remaining;
        if expanded > before {
            return Err(Failure::ArtifactInvalid);
        }
    }
    // Explicit directory/entry allocation reserve, even for zero-length files.
    let used = MAX_ENTRIES - files;
    if used
        .checked_mul(4096)
        .is_none_or(|allocation| allocation > expanded)
    {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    Ok(())
}
fn validate_extensions<R: Read>(entry: &mut tar::Entry<'_, R>) -> Result<(), Failure> {
    let kind = entry.header().entry_type();
    if (kind.is_pax_global_extensions() || kind.is_pax_local_extensions())
        && let Some(extensions) = entry
            .pax_extensions()
            .map_err(|_| Failure::ArtifactInvalid)?
    {
        for extension in extensions {
            let extension = extension.map_err(|_| Failure::ArtifactInvalid)?;
            let key = extension.key_bytes();
            if key.starts_with(b"GNU.sparse.") || key == b"SCHILY.realsize" || key == b"size" {
                return Err(Failure::ArtifactBoundsExceeded);
            }
        }
    }
    Ok(())
}
fn validate_entry<R: Read>(entry: &tar::Entry<'_, R>, path: &Path) -> Result<(), Failure> {
    let kind = entry.header().entry_type();
    let extension = kind.is_gnu_longname()
        || kind.is_gnu_longlink()
        || kind.is_pax_local_extensions()
        || kind.is_pax_global_extensions();
    if path.as_os_str().len() > 4096
        || path.components().count() > 128
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || entry.size() > MAX_FILE
        || extension && entry.size() > 65536
        || !(kind.is_file()
            || kind.is_dir()
            || kind.is_symlink()
            || kind.is_hard_link()
            || extension)
    {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    // Umoci owns standard OCI link application, including absolute container
    // symlinks. These paths are never followed by this resource preflight.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decompression_budget_fails_before_accepting_excess_bytes() {
        let mut reader = Budget {
            inner: &b"oversized"[..],
            remaining: 4,
        };
        let mut bytes = Vec::new();
        assert!(reader.read_to_end(&mut bytes).is_err());
        assert_eq!(bytes, b"over");
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn fixture(
        kind: tar::EntryType,
        size: u64,
    ) -> Result<(tempfile::TempDir, Identity), Box<dyn std::error::Error + Send + Sync>> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("blobs/sha256"))?;
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_mode(0o644);
        header.set_size(size);
        if kind.is_symlink() {
            header.set_link_name("/usr/bin/tool")?;
        }
        header.set_cksum();
        builder.append_data(&mut header, "bin/tool", io::repeat(0).take(size))?;
        let bytes = builder.into_inner()?;
        let digest = crate::oci::digest(&bytes);
        fs::write(
            root.path()
                .join("blobs/sha256")
                .join(digest.trim_start_matches("sha256:")),
            &bytes,
        )?;
        Ok((
            root,
            Identity {
                manifest_digest: String::new(),
                config_digest: String::new(),
                platform: "linux/arm64".into(),
                layers: vec![crate::oci::Descriptor {
                    digest,
                    size: u64::try_from(bytes.len())?,
                    media_type: "application/vnd.oci.image.layer.v1.tar".into(),
                }],
            },
        ))
    }
    #[test]
    fn expansion_and_filesystem_allocation_are_bounded() -> TestResult {
        let (root, identity) = fixture(tar::EntryType::Regular, 1024 * 1024)?;
        assert!(preflight(root.path(), &identity, 64 * 1024 * 1024 + 1024 * 1024).is_err());
        assert!(preflight(root.path(), &identity, 128 * 1024 * 1024).is_ok());
        Ok(())
    }
    #[test]
    fn container_absolute_symlink_is_supported_but_special_inode_is_rejected() -> TestResult {
        let (root, identity) = fixture(tar::EntryType::Symlink, 0)?;
        assert!(preflight(root.path(), &identity, 128 * 1024 * 1024).is_ok());
        let (root, identity) = fixture(tar::EntryType::Fifo, 0)?;
        assert!(preflight(root.path(), &identity, 128 * 1024 * 1024).is_err());
        Ok(())
    }
    #[test]
    fn compressed_tar_and_pax_sparse_metadata_cannot_bypass_preflight() -> TestResult {
        use std::io::Write as _;
        let (root, mut identity) = fixture(tar::EntryType::Regular, 1024 * 1024)?;
        let descriptor = &identity.layers[0];
        let plain = fs::read(
            root.path()
                .join("blobs/sha256")
                .join(descriptor.digest.trim_start_matches("sha256:")),
        )?;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&plain)?;
        let bytes = encoder.finish()?;
        let digest = crate::oci::digest(&bytes);
        fs::write(
            root.path()
                .join("blobs/sha256")
                .join(digest.trim_start_matches("sha256:")),
            &bytes,
        )?;
        identity.layers[0] = crate::oci::Descriptor {
            digest,
            size: u64::try_from(bytes.len())?,
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
        };
        assert!(preflight(root.path(), &identity, 64 * 1024 * 1024 + 512 * 1024).is_err());
        let mut tar = tar::Builder::new(Vec::new());
        tar.append_pax_extensions([("GNU.sparse.size", &b"999999999999"[..])])?;
        let bytes = tar.into_inner()?;
        let digest = crate::oci::digest(&bytes);
        fs::write(
            root.path()
                .join("blobs/sha256")
                .join(digest.trim_start_matches("sha256:")),
            &bytes,
        )?;
        identity.layers[0] = crate::oci::Descriptor {
            digest,
            size: u64::try_from(bytes.len())?,
            media_type: "application/vnd.oci.image.layer.v1.tar".into(),
        };
        assert!(preflight(root.path(), &identity, 128 * 1024 * 1024).is_err());
        Ok(())
    }
}
