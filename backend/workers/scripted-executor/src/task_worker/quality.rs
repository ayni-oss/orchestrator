//! Credential-free preparation and validation with retained launcher-owned evidence.
use super::{Result, Worker, ledger::Ledger};
use ayni_orchestrator_events::{
    task_execution::{TaskCommand, WorkerRequest, digest},
    task_quality::{MAX_ARTIFACT, QualityOutcome, snapshot_digest},
};
use ayni_orchestrator_scripted_executor::task_files;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Read as _, Write as _},
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::Path,
    process::{Command, Stdio},
};

const PRIVATE: &str = "/quality-private";
const FROZEN: &str = "/quality-frozen";
const SOURCE: &str = "/workspace";
#[derive(Clone, Serialize, Deserialize)]
struct Frozen {
    result: QualityOutcome,
    patch_digest: String,
    untracked_digest: String,
    expected_targets: u64,
}
#[derive(Default, Serialize, Deserialize)]
struct RuntimeResult {
    exit_code: Option<i32>,
    artifact_valid: bool,
    stdout: String,
    stderr: String,
    runtime_metadata_digest: String,
    executor_digest: String,
    runtime_platform: String,
}
fn read<T: serde::de::DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write(path: impl AsRef<Path>, value: &impl Serialize) -> Result {
    let path = path.as_ref();
    let temporary = path.with_extension("pending");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().ok_or("quality parent missing")?)?.sync_all()?;
    Ok(())
}
fn patch(root: &Path) -> Result<String> {
    Ok(digest(&task_files::git(
        root,
        &[
            "diff",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ],
        None,
    )?))
}

fn untracked(root: &Path) -> Result<String> {
    Ok(digest(&task_files::git(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        None,
    )?))
}
/// Independent input inventory includes prepared dependency bytes and symlink
/// targets. It never follows links and excludes only Git metadata and artifacts.
fn tree_digest(root: &Path) -> Result<String> {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    let mut remaining = 2_u64 * 1024 * 1024 * 1024;
    let mut entries = 200_000_u64;
    hash_tree(root, root, &mut hash, &mut remaining, &mut entries)?;
    Ok(format!("sha256:{:x}", hash.finalize()))
}
fn hash_tree(
    root: &Path,
    directory: &Path,
    hash: &mut sha2::Sha256,
    remaining: &mut u64,
    entries: &mut u64,
) -> Result {
    use sha2::Digest as _;
    let mut children = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        if directory == root && matches!(child.file_name().to_str(), Some(".git" | ".ayni")) {
            continue;
        }
        *entries = entries.checked_sub(1).ok_or("validation entry limit")?;
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)?;
        let relative = path.strip_prefix(root)?.as_os_str().as_encoded_bytes();
        hash.update(u64::try_from(relative.len())?.to_le_bytes());
        hash.update(relative);
        hash.update(metadata.permissions().mode().to_le_bytes());
        if metadata.is_symlink() {
            hash.update(b"link");
            let resolved = fs::canonicalize(&path)?;
            let canonical_root = fs::canonicalize(root)?;
            if !resolved.starts_with(&canonical_root)
                || resolved.starts_with(canonical_root.join(".ayni"))
                || resolved.starts_with(canonical_root.join(".git"))
            {
                return Err("mutable or external validation link".into());
            }
            let link = fs::read_link(&path)?;
            let bytes = link.as_os_str().as_encoded_bytes();
            hash.update(u64::try_from(bytes.len())?.to_le_bytes());
            hash.update(bytes);
        } else if metadata.is_dir() {
            hash.update(b"directory");
            hash_tree(root, &path, hash, remaining, entries)?;
        } else if metadata.is_file() {
            if metadata.nlink() != 1 {
                return Err("aliased validation file".into());
            }
            hash.update(b"file");
            hash.update(metadata.len().to_le_bytes());
            *remaining = remaining
                .checked_sub(metadata.len())
                .ok_or("validation byte limit")?;
            let mut file = fs::File::open(&path)?;
            let mut buffer = [0_u8; 8192];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
        } else {
            return Err("unsupported validation source entry".into());
        }
    }
    Ok(())
}

fn blocked(result: &mut QualityOutcome, reason: &str) {
    result.state = "blocked".into();
    result.safe_reason = reason.into();
}
/// The coding process has exited before this trusted stage is scheduled.
pub fn freeze(worker: &Worker, command: &TaskCommand) -> Result {
    let outcome = Ledger::open(&worker.root, command)?
        .outcome()?
        .ok_or("coding outcome missing")?;
    let root = worker
        .root
        .parent()
        .ok_or("quality root missing")?
        .join("quality");
    let frozen = root.join("frozen");
    fs::create_dir_all(&frozen)?;
    for directory in ["source/.git", "source/.ayni", "runtime", "artifacts"] {
        fs::create_dir_all(root.join(directory))?;
    }
    fs::set_permissions(root.join("runtime"), fs::Permissions::from_mode(0o770))?;
    fs::set_permissions(root.join("artifacts"), fs::Permissions::from_mode(0o777))?;
    let mut result = QualityOutcome {
        snapshot_digest: snapshot_digest(command, &outcome),
        lock_digest: command.lock_digest.clone(),
        runtime_image: command.runtime_image.clone(),
        ..Default::default()
    };
    let mut metadata = Frozen {
        result: result.clone(),
        patch_digest: String::new(),
        untracked_digest: String::new(),
        expected_targets: 0,
    };
    if outcome.state != "completed" {
        blocked(&mut result, "execution_failed");
    } else if command.runtime_image.starts_with("example.invalid/") {
        blocked(&mut result, "fixture_only");
    } else {
        prepare_snapshot(worker, command, &outcome, &root, &mut metadata, &mut result)?;
    }
    metadata.result = result;
    write(frozen.join("manifest.json"), &metadata)
}

fn prepare_snapshot(
    worker: &Worker,
    command: &TaskCommand,
    outcome: &ayni_orchestrator_events::task_execution::TaskOutcome,
    root: &Path,
    metadata: &mut Frozen,
    result: &mut QualityOutcome,
) -> Result {
    let source = root.join("source");
    if source.exists() {
        fs::remove_dir_all(&source)?;
    }
    task_files::checkout(
        &source,
        &fs::read(worker.root.join("source.pack"))?,
        command,
    )?;
    apply_patch(&source, &outcome.patch)?;
    // The index remains frozen and read-only in every untrusted runtime stage.
    let lock: Value = serde_json::from_str(&task_files::read_file(
        &source,
        ".ayni.lock",
        4 * 1024 * 1024,
    )?)?;
    if !supported_profile(&lock) {
        blocked(result, "runtime_unavailable");
    }
    result.lock_fingerprint = lock["fingerprint"]
        .as_str()
        .ok_or("lock fingerprint missing")?
        .into();
    let contract = lock["repository"]["contract_path"]
        .as_str()
        .ok_or("lock contract missing")?;
    result.contract_digest =
        digest(task_files::read_file(&source, contract, 1024 * 1024)?.as_bytes());
    metadata.expected_targets = u64::try_from(
        lock["targets"]
            .as_array()
            .ok_or("lock targets missing")?
            .len(),
    )?;
    metadata.patch_digest = patch(&source)?;
    metadata.untracked_digest = untracked(&source)?;
    writable_directories(&source)?;
    for directory in ["runtime", "artifacts"] {
        fs::create_dir_all(root.join(directory))?;
        fs::set_permissions(
            root.join(directory),
            fs::Permissions::from_mode(if directory == "runtime" { 0o770 } else { 0o777 }),
        )?;
    }
    // Empty artifact mountpoint is created before the read-only source mount.
    fs::create_dir_all(source.join(".ayni"))?;
    Ok(())
}
fn apply_patch(root: &Path, patch: &str) -> Result {
    task_files::git(
        root,
        &[
            "apply",
            "--allow-empty",
            "--index",
            "--whitespace=nowarn",
            "-",
        ],
        Some(patch.as_bytes()),
    )?;
    Ok(())
}
fn supported_profile(lock: &Value) -> bool {
    lock["capabilities"]["network"] == "none"
        && lock["capabilities"]["docker"] == "none"
        && lock["resources"]["cpus"].as_u64().is_some_and(|n| n <= 4)
        && lock["resources"]["memory_mib"]
            .as_u64()
            .is_some_and(|n| n <= 8192)
        && lock["resources"]["pids"].as_u64() == Some(2048)
        && lock["resources"]["nofile"].as_u64() == Some(8192)
}

fn writable_directories(root: &Path) -> Result {
    fs::set_permissions(root, fs::Permissions::from_mode(0o777))?;
    for item in fs::read_dir(root)? {
        let item = item?;
        if item.file_name() == ".git" {
            continue;
        }
        if item.file_type()?.is_dir() {
            writable_directories(&item.path())?;
        }
    }
    Ok(())
}
pub fn seal() -> Result {
    let manifest = Path::new(FROZEN).join("manifest.json");
    let mut frozen: Frozen = read(&manifest)?;
    if frozen.result.state == "blocked" {
        return Ok(());
    }
    let preparation: RuntimeResult = read(Path::new(PRIVATE).join("prepare.json"))?;
    if preparation.exit_code != Some(0) {
        blocked(&mut frozen.result, "runtime_unavailable");
    } else if patch(Path::new(SOURCE))? != frozen.patch_digest
        || untracked(Path::new(SOURCE))? != frozen.untracked_digest
        || digest(
            task_files::read_file(Path::new(SOURCE), ".ayni.lock", 4 * 1024 * 1024)?.as_bytes(),
        ) != frozen.result.lock_digest
    {
        blocked(&mut frozen.result, "source_changed");
    }
    if frozen.result.state != "blocked" {
        match tree_digest(Path::new(SOURCE)) {
            Ok(hash) => frozen.result.validation_input_digest = hash,
            Err(_) => blocked(&mut frozen.result, "source_changed"),
        }
    }
    write(manifest, &frozen)
}
fn executable_digest(path: &str) -> Result<String> {
    use sha2::{Digest as _, Sha256};
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}
/// This launcher retains exit status and stdout under its own UID. The child
/// cannot traverse this directory, regain privileges or inherit authority.
pub fn launch(stage: &str) -> Result {
    let frozen: Frozen = read(Path::new(FROZEN).join("manifest.json"))?;
    if frozen.result.state == "blocked" {
        return Ok(());
    }
    fs::create_dir_all(PRIVATE)?;
    let private = fs::symlink_metadata(PRIVATE)?;
    if !private.is_dir() || private.permissions().mode() & 0o007 != 0 {
        return Err("quality evidence directory is not private".into());
    }
    let destination = Path::new(PRIVATE).join(format!("{stage}.json"));
    if destination.exists() {
        return Ok(());
    }
    let metadata = fs::read("/etc/ayni/runtime.json").unwrap_or_default();
    let mut result = RuntimeResult {
        exit_code: None,
        artifact_valid: false,
        stdout: String::new(),
        stderr: String::new(),
        runtime_metadata_digest: digest(&metadata),
        executor_digest: executable_digest("/usr/local/bin/ayni").unwrap_or_default(),
        runtime_platform: format!(
            "linux/{}",
            if env::consts::ARCH == "aarch64" {
                "arm64"
            } else {
                "amd64"
            }
        ),
    };
    if stage == "quality"
        && tree_digest(Path::new(SOURCE))? != frozen.result.validation_input_digest
    {
        return write(destination, &result);
    }
    let started = Path::new(PRIVATE).join(format!("{stage}.started"));
    if started.exists() {
        return write(destination, &result);
    }
    let marker = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(started)?;
    marker.sync_all()?;
    fs::File::open(PRIVATE)?.sync_all()?;
    prepare_cache_directories()?;
    capture(stage, &mut result)?;
    if stage == "quality"
        && !result.stdout.is_empty()
        && tree_digest(Path::new(SOURCE))? == frozen.result.validation_input_digest
    {
        let artifact = Path::new(PRIVATE).join("artifact.json");
        fs::write(&artifact, result.stdout.as_bytes())?;
        // Ayni's supported reader validates its own complete serialized schema,
        // including derived aggregate and completion views. No project code runs.
        result.artifact_valid = Command::new("/usr/local/bin/ayni")
            .args(["results", "compare", "--baseline"])
            .arg(&artifact)
            .arg("--candidate")
            .arg(&artifact)
            .args(["--output", "json"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    }
    write(destination, &result)
}

fn prepare_cache_directories() -> Result {
    for path in [
        "/tmp/home",
        "/tmp/cache",
        "/tmp/cargo",
        "/tmp/target",
        "/tmp/mise",
        "/tmp/npm",
        "/tmp/gradle",
        "/tmp/uv",
    ] {
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        if !fs::symlink_metadata(path)?.is_dir() {
            return Err("invalid quality cache directory".into());
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o777))?;
    }
    Ok(())
}
fn capture(stage: &str, result: &mut RuntimeResult) -> Result {
    use std::os::unix::{fs::OpenOptionsExt as _, process::CommandExt as _};
    let stdout = Path::new(PRIVATE).join(format!("{stage}.stdout"));
    let stderr = Path::new(PRIVATE).join(format!("{stage}.stderr"));
    let open = |path: &Path| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    };
    // Regular files are intentional: Landlock cannot restrict reopening a pipe
    // through procfs, but does restrict reopening these private file inodes.
    let mut child = Command::new(env::current_exe()?)
        .args(["quality-child", stage])
        .uid(10002)
        .gid(10002)
        .current_dir(SOURCE)
        .stdin(Stdio::null())
        .stdout(open(&stdout)?)
        .stderr(open(&stderr)?)
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            result.exit_code = status.code();
            break;
        }
        if fs::metadata(&stdout)?.len() > u64::try_from(MAX_ARTIFACT)?
            || fs::metadata(&stderr)?.len() > 8192
        {
            child.kill()?;
            result.exit_code = child.wait()?.code();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    finish_capture(&stdout, &stderr, result)
}
fn finish_capture(stdout: &Path, stderr: &Path, result: &mut RuntimeResult) -> Result {
    // Recheck after wait: a short-lived producer can exit between polls.
    let oversized_stdout = bound_capture(stdout, u64::try_from(MAX_ARTIFACT)?)?;
    let oversized_stderr = bound_capture(stderr, 8192)?;
    if oversized_stdout || oversized_stderr {
        result.exit_code = None;
    }
    let mut bytes = Vec::new();
    fs::File::open(stdout)?
        .take(u64::try_from(MAX_ARTIFACT)? + 1)
        .read_to_end(&mut bytes)?;
    if !oversized_stdout && !oversized_stderr && bytes.len() <= MAX_ARTIFACT {
        result.stdout = String::from_utf8(std::mem::take(&mut bytes)).unwrap_or_default();
    } else {
        result.stdout.clear();
    }
    bytes.clear();
    fs::File::open(stderr)?.take(8192).read_to_end(&mut bytes)?;
    result.stderr = String::from_utf8_lossy(&bytes).into_owned();
    Ok(())
}

fn bound_capture(path: &Path, maximum: u64) -> Result<bool> {
    let file = fs::OpenOptions::new().write(true).open(path)?;
    if file.metadata()?.len() <= maximum {
        return Ok(false);
    }
    file.set_len(maximum)?;
    file.sync_all()?;
    Ok(true)
}

/// Runs before constructing Tokio's multithreaded runtime.
#[cfg(target_os = "linux")]
pub fn child(stage: &str) -> Result {
    use rustix::{
        process,
        thread::{self, CapabilitySet},
    };
    use std::os::unix::process::CommandExt as _;
    // CommandExt drops UID, GID and supplementary groups in the forked child,
    // while its launcher capabilities still exist, before this exec boundary.
    if process::getuid().as_raw() != 10002
        || process::geteuid().as_raw() != 10002
        || process::getgid().as_raw() != 10002
        || process::getegid().as_raw() != 10002
        || !process::getgroups()
            .map_err(|_| "quality child groups unavailable")?
            .is_empty()
    {
        return Err("quality child identity was not isolated".into());
    }
    let capabilities =
        thread::capabilities(None).map_err(|_| "quality child capabilities unavailable")?;
    if capabilities.effective != CapabilitySet::empty()
        || capabilities.permitted != CapabilitySet::empty()
        || capabilities.inheritable != CapabilitySet::empty()
    {
        return Err("quality child retained capabilities".into());
    }
    thread::set_no_new_privs(true).map_err(|_| "quality child no_new_privs failed")?;
    for (resource, limit) in [
        (rustix::process::Resource::Nproc, 2048),
        (rustix::process::Resource::Nofile, 8192),
    ] {
        rustix::process::setrlimit(
            resource,
            rustix::process::Rlimit {
                current: Some(limit),
                maximum: Some(limit),
            },
        )?;
    }
    super::network::isolate().map_err(|_| "quality child network isolation failed")?;
    super::filesystem::isolate(stage).map_err(|_| "quality child filesystem isolation failed")?;
    let mut command = Command::new("/usr/local/bin/ayni-entrypoint");
    if stage == "prepare" {
        command.arg("/bin/true");
    } else {
        command.args(["/usr/local/bin/ayni", "check", "--output", "json"]);
    }
    let error = command.current_dir(SOURCE).exec();
    Err(error.into())
}
#[cfg(not(target_os = "linux"))]
pub fn child(_: &str) -> Result {
    Err("quality privilege isolation requires Linux".into())
}

pub async fn collect(worker: &Worker) -> Result {
    let frozen: Frozen = read(Path::new(FROZEN).join("manifest.json"))?;
    let mut result = frozen.result;
    if result.safe_reason == "execution_failed" {
        return Ok(());
    }
    if result.state != "blocked" {
        let runtime: RuntimeResult = read(Path::new(PRIVATE).join("quality.json"))?;
        let artifact: Value = if runtime.artifact_valid {
            serde_json::from_str(&runtime.stdout).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        result.state = "passed".into();
        result.runtime_platform = runtime.runtime_platform;
        result.runtime_metadata_digest = runtime.runtime_metadata_digest;
        result.executor_digest = runtime.executor_digest;
        result.exit_code = runtime.exit_code;
        result.source_fingerprint = artifact["source_fingerprint"]
            .as_str()
            .unwrap_or_default()
            .into();
        result.expected_targets = frozen.expected_targets;
        result.completed_targets = artifact["completion"]["completed_targets"]
            .as_u64()
            .unwrap_or(0);
        result.artifact = runtime.stdout;
        result.artifact_digest = digest(result.artifact.as_bytes());
        if !result.validate() {
            result.state = "failed".into();
            result.safe_reason = "quality_failed".into();
            if !result.validate() {
                result = QualityOutcome {
                    state: "blocked".into(),
                    safe_reason: "evidence_invalid".into(),
                    snapshot_digest: result.snapshot_digest,
                    lock_digest: result.lock_digest,
                    runtime_image: result.runtime_image,
                    ..Default::default()
                };
            }
        }
    }
    // Persist before reporting: response loss and restarts replay exact evidence.
    let retained = worker.root.join("quality.json");
    if retained.exists() {
        if read::<QualityOutcome>(&retained)? != result {
            return Err("quality evidence conflict".into());
        }
    } else {
        write(&retained, &result)?;
    }
    for _ in 0..30 {
        if worker
            .call(&WorkerRequest::QualityReport {
                outcome: result.clone(),
            })
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(io::Error::other("quality evidence awaits acceptance").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_retains_complete_repository_reports_and_rejects_excess_output() -> Result {
        let directory = tempfile::tempdir()?;
        let stdout = directory.path().join("stdout");
        let stderr = directory.path().join("stderr");
        let report = format!("{{\"details\":\"{}\"}}", "x".repeat(256 * 1024));
        fs::write(&stdout, &report)?;
        fs::write(&stderr, "")?;
        let mut result = RuntimeResult {
            exit_code: Some(0),
            ..Default::default()
        };
        finish_capture(&stdout, &stderr, &mut result)?;
        assert_eq!(result.stdout, report);
        assert_eq!(result.exit_code, Some(0));
        fs::write(&stdout, "x".repeat(MAX_ARTIFACT + 1))?;
        finish_capture(&stdout, &stderr, &mut result)?;
        assert_eq!(result.exit_code, None);
        assert!(result.stdout.is_empty());
        assert_eq!(fs::metadata(&stdout)?.len(), u64::try_from(MAX_ARTIFACT)?);
        Ok(())
    }
    #[test]
    fn exited_producer_cannot_bypass_output_limit() -> Result {
        const CHILD: &str = "AYNI_CAPTURE_LIMIT_TEST_CHILD";
        if env::var_os(CHILD).is_some() {
            eprint!("{}", "x".repeat(8193));
            std::process::exit(0);
        }
        let directory = tempfile::tempdir()?;
        let stdout = directory.path().join("stdout");
        let stderr = directory.path().join("stderr");
        let mut child = Command::new(env::current_exe()?)
            .args([
                "--exact",
                "quality::tests::exited_producer_cannot_bypass_output_limit",
                "--nocapture",
            ])
            .env(CHILD, "true")
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout)?)
            .stderr(fs::File::create(&stderr)?)
            .spawn()?;
        let status = child.wait()?;
        assert!(status.success());
        assert!(fs::metadata(&stderr)?.len() > 8192);
        let mut result = RuntimeResult {
            exit_code: status.code(),
            ..Default::default()
        };
        finish_capture(&stdout, &stderr, &mut result)?;
        assert_eq!(result.exit_code, None);
        assert!(result.stdout.is_empty());
        assert_eq!(result.stderr.len(), 8192);
        assert_eq!(fs::metadata(&stderr)?.len(), 8192);
        Ok(())
    }
    #[test]
    fn completed_empty_patch_preserves_source_and_index() -> Result {
        let root = tempfile::tempdir()?;
        task_files::git(root.path(), &["init", "--initial-branch=task"], None)?;
        fs::write(root.path().join("source.rs"), "pub fn unchanged() {}\n")?;
        task_files::git(root.path(), &["add", "--", "source.rs"], None)?;
        let index = fs::read(root.path().join(".git/index"))?;
        let source = fs::read(root.path().join("source.rs"))?;
        apply_patch(root.path(), "")?;
        assert_eq!(fs::read(root.path().join(".git/index"))?, index);
        assert_eq!(fs::read(root.path().join("source.rs"))?, source);
        Ok(())
    }
    #[test]
    fn frozen_inventory_covers_bytes_modes_links_and_untracked_inputs() -> Result {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("source"), "before")?;
        let original = tree_digest(root.path())?;
        fs::write(root.path().join("source"), "after")?;
        assert_ne!(tree_digest(root.path())?, original);
        let content = tree_digest(root.path())?;
        fs::set_permissions(
            root.path().join("source"),
            fs::Permissions::from_mode(0o755),
        )?;
        assert_ne!(tree_digest(root.path())?, content);
        let executable = tree_digest(root.path())?;
        fs::write(root.path().join("untracked"), "new")?;
        assert_ne!(tree_digest(root.path())?, executable);
        let untracked = tree_digest(root.path())?;
        std::os::unix::fs::symlink("source", root.path().join("link"))?;
        assert_ne!(tree_digest(root.path())?, untracked);
        let linked = tree_digest(root.path())?;
        fs::create_dir_all(root.path().join(".ayni/last"))?;
        fs::write(root.path().join(".ayni/last/signals.json"), "untrusted")?;
        assert_eq!(tree_digest(root.path())?, linked);
        fs::hard_link(root.path().join("source"), root.path().join(".ayni/alias"))?;
        assert!(tree_digest(root.path()).is_err());
        fs::remove_file(root.path().join(".ayni/alias"))?;
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("mutable"), "changing")?;
        std::os::unix::fs::symlink(outside.path().join("mutable"), root.path().join("external"))?;
        assert!(tree_digest(root.path()).is_err());
        fs::remove_file(root.path().join("external"))?;
        std::os::unix::fs::symlink(".ayni/last/signals.json", root.path().join("evidence-link"))?;
        assert!(tree_digest(root.path()).is_err());
        Ok(())
    }
}
