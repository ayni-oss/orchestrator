use super::{Expected, Failure, PREPARED, Prepared, SOURCE, WORKSPACE, canonical_digest};
use ayni_core::sha256_fingerprint;
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};
const MAX_FILE: u64 = 32 * 1024 * 1024;
#[cfg(unix)]
fn marker_directory() -> Result<(), Failure> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let directory = Path::new(PREPARED)
        .parent()
        .ok_or(Failure::ArtifactInvalid)?;
    let metadata = fs::symlink_metadata(directory).map_err(|_| Failure::ArtifactInvalid)?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(Failure::ArtifactInvalid);
    }
    Ok(())
}
#[cfg(unix)]
pub(super) fn write_prepared(marker: &Prepared) -> Result<(), Failure> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    fs::DirBuilder::new()
        .mode(0o700)
        .create(
            Path::new(PREPARED)
                .parent()
                .ok_or(Failure::ArtifactInvalid)?,
        )
        .map_err(|_| Failure::ArtifactConflict)?;
    marker_directory()?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(PREPARED)
        .map_err(|_| Failure::ArtifactConflict)?;
    file.write_all(&serde_json::to_vec(marker).map_err(|_| Failure::ArtifactInvalid)?)
        .map_err(|_| Failure::ExecutionFailed)?;
    file.sync_all().map_err(|_| Failure::ExecutionFailed)
}
#[cfg(unix)]
pub(super) fn read_prepared(expected: &Expected) -> Result<Prepared, Failure> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    marker_directory()?;
    let metadata = fs::symlink_metadata(PREPARED).map_err(|_| Failure::ArtifactInvalid)?;
    if metadata.uid() != 0 || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(Failure::ArtifactInvalid);
    }
    let marker: Prepared = serde_json::from_slice(&bounded_file(Path::new(PREPARED), 4096)?)
        .map_err(|_| Failure::ArtifactInvalid)?;
    if marker.source_digest != expected.source
        || marker.lock_fingerprint != expected.lock
        || marker.certificate_digest != expected.certificate
        || marker.platform != expected.platform
        || !canonical_digest(&marker.tree_digest)
    {
        return Err(Failure::SourceIdentityMismatch);
    }
    Ok(marker)
}

#[cfg(unix)]
pub(super) fn copy_checkout() -> Result<(), Failure> {
    use std::os::unix::fs::PermissionsExt as _;
    if fs::read_dir(WORKSPACE)
        .map_err(|_| Failure::SourceUnavailable)?
        .next()
        .is_some()
    {
        return Err(Failure::ArtifactConflict);
    }
    for relative in original_paths(Path::new(SOURCE))? {
        let source = Path::new(SOURCE).join(&relative);
        let target = Path::new(WORKSPACE).join(&relative);
        fs::create_dir_all(target.parent().ok_or(Failure::SourcePathUnsupported)?)
            .map_err(|_| Failure::ExecutionFailed)?;
        let bytes = bounded_file(&source, MAX_FILE)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|_| Failure::ArtifactConflict)?;
        file.write_all(&bytes)
            .map_err(|_| Failure::ExecutionFailed)?;
        fs::set_permissions(
            &target,
            fs::Permissions::from_mode(
                fs::symlink_metadata(&source)
                    .map_err(|_| Failure::SourceUnavailable)?
                    .permissions()
                    .mode()
                    & 0o777,
            ),
        )
        .map_err(|_| Failure::ExecutionFailed)?;
    }
    for path in [
        "/workspace/.ayni",
        "/tmp/home",
        "/tmp/cache",
        "/tmp/mise",
        "/tmp/cargo",
        "/tmp/target",
        "/tmp/npm",
        "/tmp/gradle",
        "/tmp/uv",
    ] {
        fs::create_dir_all(path).map_err(|_| Failure::ExecutionFailed)?;
    }
    Ok(())
}
fn original_paths(root: &Path) -> Result<Vec<PathBuf>, Failure> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|_| Failure::SourceUnavailable)? {
            entries += 1;
            if entries > 20_000 {
                return Err(Failure::SourceBoundsExceeded);
            }
            let path = entry.map_err(|_| Failure::SourceUnavailable)?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::SourceUnavailable)?;
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                files.push(
                    path.strip_prefix(root)
                        .map_err(|_| Failure::SourcePathUnsupported)?
                        .to_owned(),
                );
            } else {
                return Err(Failure::SourcePathUnsupported);
            }
        }
    }
    Ok(files)
}
#[cfg(unix)]
pub(super) fn compare_originals(source: &Path, workspace: &Path) -> Result<(), Failure> {
    use std::os::unix::fs::PermissionsExt as _;
    for relative in original_paths(source)? {
        original_ancestors(workspace, &relative)?;
        let original = source.join(&relative);
        let actual = workspace.join(relative);
        let metadata =
            fs::symlink_metadata(&actual).map_err(|_| Failure::SourceIdentityMismatch)?;
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o777
                != fs::symlink_metadata(&original)
                    .map_err(|_| Failure::SourceUnavailable)?
                    .permissions()
                    .mode()
                    & 0o777
            || file_digest(&original, MAX_FILE)? != file_digest(&actual, MAX_FILE)?
        {
            return Err(Failure::SourceIdentityMismatch);
        }
    }
    Ok(())
}
fn original_ancestors(workspace: &Path, relative: &Path) -> Result<(), Failure> {
    for ancestor in relative.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(workspace.join(ancestor))
            .map_err(|_| Failure::SourceIdentityMismatch)?;
        if !metadata.is_dir() {
            return Err(Failure::SourceIdentityMismatch);
        }
    }
    Ok(())
}

pub(super) fn bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, Failure> {
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
pub(super) fn file_digest(path: &Path, maximum: u64) -> Result<String, Failure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Failure::ArtifactInvalid)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(Failure::ArtifactBoundsExceeded);
    }
    let mut source = fs::File::open(path)
        .map_err(|_| Failure::ArtifactInvalid)?
        .take(maximum + 1);
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 65_536].into_boxed_slice();
    let mut total = 0_u64;
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| Failure::ArtifactInvalid)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > maximum {
            return Err(Failure::ArtifactBoundsExceeded);
        }
        hash.update(&buffer[..count]);
    }
    if total != metadata.len() {
        return Err(Failure::SourceIdentityMismatch);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

#[cfg(unix)]
pub(super) fn prepared_digest(root: &Path) -> Result<String, Failure> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut pending = vec![root.to_owned()];
    let mut entries = std::collections::BTreeMap::new();
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|_| Failure::SourceUnavailable)? {
            let path = entry.map_err(|_| Failure::SourceUnavailable)?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| Failure::SourcePathUnsupported)?;
            if matches!(
                relative
                    .components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str()),
                Some(".ayni" | ".git")
            ) {
                continue;
            }
            if entries.len() >= 100_000
                || relative.components().count() > 128
                || relative.as_os_str().len() > 4096
            {
                return Err(Failure::ArtifactBoundsExceeded);
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::SourceUnavailable)?;
            let mode = metadata.permissions().mode() & 0o7777;
            let identity = if metadata.is_dir() {
                pending.push(path.clone());
                "directory".into()
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
                if total > 8 * 1024 * 1024 * 1024 {
                    return Err(Failure::ArtifactBoundsExceeded);
                }
                file_digest(&path, 256 * 1024 * 1024)?
            } else if metadata.file_type().is_symlink() {
                let target = fs::read_link(&path).map_err(|_| Failure::ArtifactInvalid)?;
                let value = target.to_str().ok_or(Failure::SourcePathUnsupported)?;
                if value.len() > 4096 {
                    return Err(Failure::ArtifactBoundsExceeded);
                }
                format!("link:{value}")
            } else {
                return Err(Failure::SourcePathUnsupported);
            };
            entries.insert(
                relative
                    .to_str()
                    .ok_or(Failure::SourcePathUnsupported)?
                    .to_owned(),
                (mode, identity),
            );
        }
    }
    let bytes = serde_json::to_vec(&entries).map_err(|_| Failure::ArtifactInvalid)?;
    Ok(sha256_fingerprint(bytes))
}

pub fn install_probes() -> Result<(), Failure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for (name, source) in [
            ("runtime-worker", "/runtime-worker"),
            ("ayni-runtime-verifier", "/ayni-runtime-verifier"),
        ] {
            let bytes = bounded_file(Path::new(source), 256 * 1024 * 1024)?;
            let path = Path::new("/trusted").join(name);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| Failure::ArtifactConflict)?;
            file.write_all(&bytes)
                .map_err(|_| Failure::ExecutionFailed)?;
            file.set_permissions(fs::Permissions::from_mode(0o755))
                .map_err(|_| Failure::ExecutionFailed)?;
            file.sync_all().map_err(|_| Failure::ExecutionFailed)?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(Failure::InvalidInput)
    }
}
