use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

/// Frozen, credential-free execution request mounted read-only by Kubernetes.
///
/// A profile supplies its explicit admission expectations. The shared request
/// does not require a particular tool, certification policy, or assessment model.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest<Profile> {
    pub repository_url: String,
    pub revision: String,
    pub expected: Profile,
}

/// Read a bounded ordinary file, rejecting symlinks and special files.
///
/// # Errors
/// Returns an error when the file is unsafe, inaccessible, or oversized.
pub fn read_bounded(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(io::Error::other("invalid or oversized runtime input"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::other("oversized runtime input"));
    }
    Ok(bytes)
}
