//! Atomic recovery generations preserve local history and source without commits.
use super::sessions::SessionState;
use crate::operations::runtime::git;
use ayni_orchestrator_profile_ayni::coding::WorkerSpec;
use ayni_orchestrator_runtime::runtime_execution::read_bounded;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 131_072 {
        return Err(io::Error::other("worker output exceeds bound"));
    }
    atomic_write(path, &bytes)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("output parent missing"))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&read_bounded(path, 131_072)?).map_err(io::Error::other)
}

pub fn save(
    spec: &WorkerSpec,
    workspace: &Path,
    output: &Path,
    state: &SessionState,
) -> io::Result<()> {
    super::commits::changed_paths(spec, workspace)?;
    let root = output.join("recovery");
    fs::create_dir_all(&root)?;
    let pending = tempfile::Builder::new()
        .prefix("generation-")
        .tempdir_in(&root)?;
    let source = pending.path().join("workspace");
    fs::create_dir(&source)?;
    let mut size = 0;
    copy_tree(
        &workspace.join(".git"),
        &source.join(".git"),
        &mut size,
        false,
    )?;
    let files = git(
        workspace,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    for path in files.split('\0').filter(|path| !path.is_empty()) {
        if !ayni_orchestrator_domain::repository_path_is_valid(path) {
            return Err(io::Error::other("unsafe recovery path"));
        }
        if path.starts_with(".ayni/") || path.starts_with(".orchestrator/") {
            continue;
        }
        let from = workspace.join(path);
        if from.symlink_metadata().is_ok() {
            copy_tree(&from, &source.join(path), &mut size, true)?;
        }
    }
    write_json(&pending.path().join("state.json"), state)?;
    write_json(&pending.path().join("identity.json"), &identity(spec)?)?;
    let generation = pending
        .path()
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| io::Error::other("generation identity missing"))?
        .to_owned();
    sync_directories(pending.path())?;
    let _retained = pending.keep();
    atomic_write(&output.join("recovery-current"), generation.as_bytes())?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name() != generation.as_str() && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

pub fn restore(
    spec: &WorkerSpec,
    workspace: &Path,
    output: &Path,
) -> io::Result<Option<SessionState>> {
    let pointer = output.join("recovery-current");
    if !pointer.exists() {
        return Ok(None);
    }
    let generation = String::from_utf8(read_bounded(&pointer, 128)?).map_err(io::Error::other)?;
    if !generation.starts_with("generation-")
        || !generation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(io::Error::other("invalid recovery pointer"));
    }
    let source = output.join("recovery").join(&generation);
    if read_json::<String>(&source.join("identity.json"))? != identity(spec)? {
        return Err(io::Error::other("recovery belongs to another frozen task"));
    }
    let mut state: SessionState = read_json(&source.join("state.json"))?;
    if let Ok(current) = read_json::<SessionState>(&output.join("session.json")) {
        // A model budget is consumed durably before issuing the provider request.
        // A crash may lose the last tool's edits but can never refund a turn.
        state.repair_stopped |= current.repair_stopped;
        state.progress.coding_turns_consumed = state
            .progress
            .coding_turns_consumed
            .max(current.progress.coding_turns_consumed);
        state.progress.quality_round = state
            .progress
            .quality_round
            .max(current.progress.quality_round);
    }
    if workspace.exists() {
        for entry in fs::read_dir(workspace)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
    }
    let mut size = 0;
    copy_tree(&source.join("workspace"), workspace, &mut size, true)?;
    Ok(Some(state))
}

fn identity(spec: &WorkerSpec) -> io::Result<String> {
    let mut frozen = spec.clone();
    frozen.publication_only = false;
    Ok(ayni_orchestrator_domain::digest::sha256(
        &serde_json::to_vec(&frozen)?,
    ))
}

fn sync_directories(directory: &Path) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directories(&entry.path())?;
        }
    }
    File::open(directory)?.sync_all()
}
fn copy_tree(source: &Path, target: &Path, bytes: &mut u64, allow_symlink: bool) -> io::Result<()> {
    let metadata = source.symlink_metadata()?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if metadata.file_type().is_symlink() {
        if !allow_symlink {
            return Err(io::Error::other("Git recovery cannot contain symlinks"));
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
        #[cfg(not(unix))]
        return Err(io::Error::other("symlink recovery unsupported"));
    } else if metadata.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &target.join(entry.file_name()),
                bytes,
                allow_symlink,
            )?;
        }
    } else if metadata.is_file() {
        *bytes = bytes.saturating_add(metadata.len());
        if *bytes > 268_435_456 {
            return Err(io::Error::other("recovery source exceeds bound"));
        }
        fs::copy(source, target)?;
        File::open(target)?.sync_all()?;
    } else {
        return Err(io::Error::other("special recovery files are unsupported"));
    }
    Ok(())
}
