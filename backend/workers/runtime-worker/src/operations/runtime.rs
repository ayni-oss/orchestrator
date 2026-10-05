//! Trusted launch operations for a mounted checkout and an admitted runtime.

use ayni_orchestrator_runtime::runtime_execution::ExecutionRequest;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
};

/// Checkout an exact GitHub commit in an init container. The optional token is
/// used only by Git's fetch process and never persisted in the checkout.
///
/// # Errors
/// Returns a bounded error on invalid input or failed checkout.
pub fn checkout<P>(
    request: &ExecutionRequest<P>,
    workspace: &Path,
    token: Option<&str>,
) -> io::Result<()> {
    validate_source(request)?;
    fs::create_dir_all(workspace)?;
    git(workspace, &["init", "--quiet"])?;
    let mut fetch = command("git", workspace);
    fetch.args([
        "-c",
        "core.hooksPath=/dev/null",
        "fetch",
        "--quiet",
        "--depth=1",
        "--no-tags",
        "--",
        &request.repository_url,
        &request.revision,
    ]);
    if let Some(token) = token {
        if token.is_empty() || token.contains(['\r', '\n']) {
            return Err(io::Error::other("invalid source credential"));
        }
        fetch
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_1", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_1",
                format!(
                    "AUTHORIZATION: basic {}",
                    STANDARD.encode(format!("x-access-token:{token}"))
                ),
            );
    }
    success(&mut fetch)?;
    git(
        workspace,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "--detach",
            "FETCH_HEAD",
        ],
    )?;
    if git(workspace, &["rev-parse", "HEAD"])? != request.revision {
        return Err(io::Error::other("source revision mismatch"));
    }
    Ok(())
}

pub(crate) fn validate_source<P>(request: &ExecutionRequest<P>) -> io::Result<()> {
    let path = request
        .repository_url
        .strip_prefix("https://github.com/")
        .and_then(|value| value.strip_suffix(".git"))
        .ok_or_else(|| io::Error::other("unsupported repository URL"))?;
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
        || request.revision.len() != 40
        || !request
            .revision
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(io::Error::other("invalid source identity"));
    }
    Ok(())
}

pub(crate) fn command(program: &str, workspace: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(workspace)
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        )
        .env("HOME", "/scratch")
        .env("XDG_CACHE_HOME", "/scratch/cache")
        .env("CARGO_HOME", "/scratch/cargo")
        .env("MISE_DATA_DIR", "/opt/ayni/mise")
        .env("MISE_TRUSTED_CONFIG_PATHS", "/etc/ayni")
        .env("MISE_AUTO_INSTALL", "0")
        .env("MISE_CACHE_DIR", "/scratch/mise-cache")
        .env("MISE_STATE_DIR", "/scratch/mise-state")
        .env(
            "RUSTUP_HOME",
            std::env::var("RUSTUP_HOME").unwrap_or_else(|_| "/home/ayni/.rustup".into()),
        )
        .env("TMPDIR", "/scratch")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // Kubernetes emptyDir roots are root-owned, even with fsGroup access.
        // Trust only this assigned checkout, including Git invoked by Ayni.
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "safe.directory")
        .env("GIT_CONFIG_VALUE_0", workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(crate) fn success(command: &mut Command) -> io::Result<()> {
    if crate::operations::process::spawn_sync(command)?
        .wait(std::time::Duration::from_secs(180))?
        .success()
    {
        Ok(())
    } else {
        Err(io::Error::other("runtime command failed"))
    }
}

pub(crate) fn git(workspace: &Path, arguments: &[&str]) -> io::Result<String> {
    let mut git_process = command("git", workspace);
    git_process
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::operations::process::spawn_sync(&mut git_process)?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("git stderr missing"))?;
    std::thread::scope(|scope| {
        // Drain both pipes concurrently; retain only a bounded diagnostic prefix.
        let diagnostic = scope.spawn(|| bounded_diagnostic(stderr));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("git stdout missing"))?;
        let output = scope.spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(1_048_577).read_to_end(&mut bytes)?;
            if bytes.len() > 1_048_576 {
                return Err(io::Error::other("git output oversized"));
            }
            Ok(bytes)
        });
        let status = child.wait(std::time::Duration::from_secs(180))?;
        let bytes = output
            .join()
            .map_err(|_| io::Error::other("git reader failed"))??;
        let diagnostic = diagnostic.join().unwrap_or_default();
        if !status.success() {
            let operation = arguments
                .iter()
                .copied()
                .find(|value| {
                    matches!(
                        *value,
                        "read-tree"
                            | "reset"
                            | "add"
                            | "diff"
                            | "commit"
                            | "bundle"
                            | "rev-parse"
                            | "fetch"
                            | "checkout"
                            | "init"
                            | "ls-tree"
                            | "ls-files"
                            | "show"
                            | "rm"
                    )
                })
                .unwrap_or("source");
            let failure = git_failure(operation, &diagnostic);
            tracing::error!(operation, exit_code = status.code(), failure = ?failure, "git operation failed");
            return Err(io::Error::other(GitFailure(failure)));
        }
        String::from_utf8(bytes)
            .map(|value| value.trim_end_matches('\n').to_owned())
            .map_err(io::Error::other)
    })
}

#[derive(Debug)]
pub(crate) struct GitFailure(pub ayni_orchestrator_runtime::activity::Failure);
impl std::fmt::Display for GitFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.message())
    }
}
impl std::error::Error for GitFailure {}

fn bounded_diagnostic(mut reader: impl Read) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let keep = read.min(8192_usize.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
    }
    retained
}

fn git_failure(operation: &str, bytes: &[u8]) -> ayni_orchestrator_runtime::activity::Failure {
    use ayni_orchestrator_runtime::activity::Failure;
    let diagnostic = String::from_utf8_lossy(bytes);
    if diagnostic.contains("Permission denied") || diagnostic.contains("Read-only file system") {
        return Failure::GitPermissions;
    }
    if diagnostic.contains("index.lock") && diagnostic.contains("File exists") {
        return Failure::GitIndexLocked;
    }
    if diagnostic.contains("ignored by one of your .gitignore files") {
        return Failure::GitIgnored;
    }
    if diagnostic.contains("Author identity unknown") {
        return Failure::GitIdentity;
    }
    match operation {
        "read-tree" | "reset" => Failure::GitIndex,
        "add" => Failure::GitStage,
        "diff" => Failure::GitDiff,
        "commit" => Failure::GitCommit,
        "bundle" => Failure::GitBundle,
        _ => Failure::GitSource,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_failure_retains_safe_operation_without_raw_diagnostics() -> io::Result<()> {
        let workspace = tempfile::tempdir()?;
        git(workspace.path(), &["init", "--quiet"])?;
        let error = git(workspace.path(), &["read-tree", "private-missing-revision"])
            .err()
            .ok_or_else(|| io::Error::other("expected failure"))?;
        assert!(matches!(
            crate::coding::activity::io_failure(&error),
            ayni_orchestrator_runtime::activity::Failure::GitIndex
        ));
        assert!(!error.to_string().contains("private-missing-revision"));
        assert_eq!(
            bounded_diagnostic(vec![b'x'; 100_000].as_slice()).len(),
            8192
        );
        assert!(matches!(
            git_failure("add", b"private-file: Permission denied"),
            ayni_orchestrator_runtime::activity::Failure::GitPermissions
        ));
        Ok(())
    }

    #[test]
    fn git_trusts_only_the_assigned_workspace() -> io::Result<()> {
        let workspace = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        git(workspace.path(), &["init", "--quiet"])?;
        git(other.path(), &["init", "--quiet"])?;
        let mut trusted = command("git", workspace.path());
        trusted
            .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
            .args(["rev-parse", "--show-toplevel"]);
        assert!(trusted.status()?.success());
        let mut untrusted = command("git", workspace.path());
        untrusted
            .current_dir(other.path())
            .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
            .args(["rev-parse", "--show-toplevel"]);
        assert!(!untrusted.status()?.success());
        Ok(())
    }

    #[test]
    fn rejects_untrusted_checkout_transport_and_revision() {
        let mut request = ExecutionRequest {
            repository_url: "https://github.com/team/repo.git".into(),
            revision: "a".repeat(40),
            expected: (),
        };
        assert!(validate_source(&request).is_ok());
        request.repository_url = "https://github.com@attacker.test/team/repo.git".into();
        assert!(validate_source(&request).is_err());
        request.repository_url = "https://github.com/team/repo.git".into();
        request.revision = "main".into();
        assert!(validate_source(&request).is_err());
    }
}
