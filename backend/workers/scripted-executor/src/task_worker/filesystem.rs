//! Prevent project descendants from rewriting the Ayni process through procfs.
use std::io;

/// Confine newly opened writable files before running project preparation/checks.
/// Existing standard streams remain usable; the launcher passes no other handles.
#[cfg(target_os = "linux")]
pub fn isolate(stage: &str) -> io::Result<()> {
    isolate_workspace(stage, std::path::Path::new("/workspace"))
}

#[cfg(target_os = "linux")]
fn isolate_workspace(stage: &str, root: &std::path::Path) -> io::Result<()> {
    use landlock::{
        AccessFs, CompatLevel, Compatible as _, PathBeneath, PathFd, Ruleset, RulesetAttr as _,
        RulesetCreatedAttr as _, RulesetStatus,
    };
    let mut rules = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::WriteFile | AccessFs::Refer)
        .map_err(io::Error::other)?
        .create()
        .map_err(io::Error::other)?;
    let workspace = match stage {
        "prepare" => root.to_path_buf(),
        "quality" => root.join(".ayni"),
        _ => return Err(io::Error::other("unknown quality isolation stage")),
    };
    for path in [std::path::Path::new("/tmp"), workspace.as_path()] {
        rules = rules
            .add_rule(PathBeneath::new(
                PathFd::new(path).map_err(io::Error::other)?,
                AccessFs::WriteFile | AccessFs::Refer,
            ))
            .map_err(io::Error::other)?;
    }
    // Devices cannot carry directory-only Refer rights.
    rules = rules
        .add_rule(PathBeneath::new(
            PathFd::new("/dev/null").map_err(io::Error::other)?,
            AccessFs::WriteFile,
        ))
        .map_err(io::Error::other)?;
    let status = rules.restrict_self().map_err(io::Error::other)?;
    if status.ruleset != RulesetStatus::FullyEnforced || !status.no_new_privs {
        return Err(io::Error::other("filesystem isolation unavailable"));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn isolate(_: &str) -> io::Result<()> {
    Err(io::Error::other(
        "quality isolation requires Linux Landlock",
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        fs::OpenOptions,
        process::{Command, Stdio},
    };

    #[test]
    fn filesystem_jail_denies_process_memory_and_allows_ephemeral_outputs() -> io::Result<()> {
        const CHILD: &str = "AYNI_FILESYSTEM_ISOLATION_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            // Self-memory is writable under normal DAC/Yama policy. Denying it
            // proves procfs cannot bypass the write allowlist to rewrite Ayni.
            let baseline = OpenOptions::new()
                .write(true)
                .open("/proc/self/mem")
                .map_err(|e| io::Error::new(e.kind(), format!("baseline proc memory: {e}")))?;
            drop(baseline);
            // Linux tmpfs mirrors the production PVC. Host VirtioFS bind
            // mounts may not preserve Landlock's inode ancestry semantics.
            let workspace = tempfile::tempdir_in("/dev/shm")?;
            std::fs::create_dir(workspace.path().join(".ayni"))?;
            isolate_workspace("prepare", workspace.path())
                .map_err(|e| io::Error::new(e.kind(), format!("prepare restriction: {e}")))?;
            let denied = OpenOptions::new()
                .write(true)
                .open("/proc/self/mem")
                .err()
                .ok_or_else(|| io::Error::other("process memory remained writable"))?;
            assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
            let stdout = OpenOptions::new()
                .write(true)
                .open("/proc/self/fd/1")
                .err()
                .ok_or_else(|| io::Error::other("Ayni stdout remained reopenable"))?;
            assert_eq!(stdout.kind(), io::ErrorKind::PermissionDenied);
            let temporary = tempfile::tempdir_in(workspace.path())
                .map_err(|e| io::Error::new(e.kind(), format!("temporary directory: {e}")))?;
            let source_directory = tempfile::tempdir_in(workspace.path())?;
            let source = source_directory.path().join("source");
            std::fs::write(&source, b"prepared")
                .map_err(|e| io::Error::new(e.kind(), format!("prepared source write: {e}")))?;
            std::fs::write(temporary.path().join("result"), b"allowed")
                .map_err(|e| io::Error::new(e.kind(), format!("allowed output write: {e}")))?;
            // Ayni materializes dependencies with cross-directory atomic rename.
            // Landlock denies this with EXDEV unless Refer is explicitly handled.
            let before = temporary.path().join("before");
            let after = temporary.path().join("after");
            std::fs::create_dir_all(&before)?;
            std::fs::create_dir_all(&after)?;
            std::fs::write(before.join("seed"), b"dependency")?;
            std::fs::rename(before.join("seed"), after.join("seed"))
                .map_err(|e| io::Error::new(e.kind(), format!("dependency seed rename: {e}")))?;
            assert_eq!(std::fs::read(after.join("seed"))?, b"dependency");
            // A second, tighter layer models final validation and retains the
            // same procfs denial without weakening the first restriction.
            isolate_workspace("quality", workspace.path())
                .map_err(|e| io::Error::new(e.kind(), format!("quality restriction: {e}")))?;
            let denied_source = std::fs::write(&source, b"changed")
                .err()
                .ok_or_else(|| io::Error::other("sealed source remained writable"))?;
            assert_eq!(denied_source.kind(), io::ErrorKind::PermissionDenied);
            let artifact = tempfile::NamedTempFile::new_in(workspace.path().join(".ayni"))?;
            std::fs::write(artifact.path(), b"artifact")
                .map_err(|e| io::Error::new(e.kind(), format!("artifact write: {e}")))?;
            drop(artifact);
            drop(source_directory);
            let denied = OpenOptions::new()
                .write(true)
                .open("/proc/self/mem")
                .err()
                .ok_or_else(|| io::Error::other("process memory remained writable"))?;
            assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
            return Ok(());
        }
        // Only the dedicated child is restricted; the shared harness is untouched.
        // A regular report file must be outside every project-write allowlist.
        // Anonymous pipes are not covered by Landlock filesystem restrictions.
        let private = tempfile::tempdir_in("/dev/shm")?;
        let report = private.path().join("report");
        let output = std::fs::File::create(&report)?;
        let status = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "filesystem::tests::filesystem_jail_denies_process_memory_and_allows_ephemeral_outputs",
            ])
            .env(CHILD, "true")
            .stdout(Stdio::from(output))
            .status()?;
        assert!(status.success(), "{}", std::fs::read_to_string(&report)?);
        Ok(())
    }
}
