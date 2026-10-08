//! Trusted launch operations for a mounted checkout and an admitted runtime.

use ayni_orchestrator_runtime::runtime_execution::ExecutionRequest;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
};

/// Checkout an exact GitHub commit after worker runtime attestation. The optional token is
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

/// Checkout a frozen source with one deadline shared by all Git subprocesses.
/// Dropping this future stops the active subprocess group and its descendants.
///
/// # Errors
/// Rejects invalid source/credentials, expired deadlines, bounded-output violations,
/// failed Git operations and a checkout that differs from the frozen source SHA.
pub async fn checkout_with_deadline<P: Sync>(
    request: &ExecutionRequest<P>,
    workspace: &Path,
    token: Option<&str>,
    deadline: std::time::Duration,
) -> io::Result<()> {
    validate_source(request)?;
    if token.is_some_and(|value| value.is_empty() || value.contains(['\r', '\n'])) {
        return Err(io::Error::other("invalid source credential"));
    }
    let expires = tokio::time::Instant::now()
        .checked_add(deadline)
        .ok_or_else(|| io::Error::other("checkout deadline outside clock range"))?;
    if deadline.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "frozen checkout deadline exhausted",
        ));
    }
    fs::create_dir_all(workspace)?;
    let mut init = command("git", workspace);
    init.args(["-c", "core.hooksPath=/dev/null", "init", "--quiet"]);
    checkout_step(init, expires, "init").await?;
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
    checkout_step(fetch, expires, "fetch").await?;
    let mut detach = command("git", workspace);
    detach.args([
        "-c",
        "core.hooksPath=/dev/null",
        "checkout",
        "--quiet",
        "--detach",
        "FETCH_HEAD",
    ]);
    checkout_step(detach, expires, "checkout").await?;
    let mut head = command("git", workspace);
    head.args(["rev-parse", "HEAD"]);
    let head = checkout_step(head, expires, "rev-parse").await?;
    if std::str::from_utf8(&head).map_err(io::Error::other)?.trim() != request.revision {
        return Err(io::Error::other("source revision mismatch"));
    }
    Ok(())
}

async fn checkout_step(
    mut command: Command,
    expires: tokio::time::Instant,
    operation: &'static str,
) -> io::Result<Vec<u8>> {
    if tokio::time::Instant::now() >= expires {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "frozen checkout deadline exhausted",
        ));
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut command = tokio::process::Command::from(command);
    let mut process = crate::operations::process::spawn(&mut command)?;
    let stdout = process
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("checkout stdout missing"))?;
    let stderr = process
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("checkout stderr missing"))?;
    let (status, stdout, stderr) = tokio::time::timeout_at(expires, async {
        tokio::join!(
            process.wait(),
            drain_checkout_output(stdout),
            drain_checkout_output(stderr)
        )
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "frozen checkout deadline exhausted",
        )
    })?;
    let stdout = stdout?;
    // Classify bounded diagnostics without returning provider details or credentials.
    let diagnostic = stderr?;
    let status = status?;
    if !status.success() {
        let failure = git_failure(operation, &diagnostic);
        tracing::error!(operation, exit_code = status.code(), failure = ?failure, "source checkout Git operation failed");
        return Err(io::Error::other(GitFailure(failure)));
    }
    Ok(stdout)
}

async fn drain_checkout_output<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
) -> io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt as _;
    let mut output = Vec::new();
    let mut oversized = false;
    let mut chunk = vec![0; 8192];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        if output.len() + count > 1_048_576 {
            oversized = true;
        } else if !oversized {
            output.extend_from_slice(&chunk[..count]);
        }
    }
    if oversized {
        return Err(io::Error::other("source checkout output exceeds bound"));
    }
    Ok(output)
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
        .env("BUILDX_CONFIG", "/scratch/buildx")
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
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
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
    #[cfg(all(target_os = "linux", not(test)))]
    let mut git_process = super::sandbox::command("git", workspace)?;
    #[cfg(any(not(target_os = "linux"), test))]
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

#[cfg(test)]
mod checkout_deadline_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn exhausted_checkout_deadline_never_initializes_source() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let workspace = directory.path().join("checkout");
        let request = ExecutionRequest {
            repository_url: "https://github.com/team/repository.git".into(),
            revision: "a".repeat(40),
            expected: (),
        };
        let error = checkout_with_deadline(&request, &workspace, None, Duration::ZERO)
            .await
            .err()
            .ok_or_else(|| io::Error::other("expected timeout"))?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(!workspace.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn checkout_steps_share_one_deadline_instead_of_resetting_it() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let expires = tokio::time::Instant::now() + Duration::from_millis(500);
        let mut first = command("/bin/sh", directory.path());
        first.args(["-c", "sleep 0.35"]);
        checkout_step(first, expires, "init").await?;
        let mut second = command("/bin/sh", directory.path());
        second.args(["-c", "sleep 0.35"]);
        let error = checkout_step(second, expires, "fetch")
            .await
            .err()
            .ok_or_else(|| io::Error::other("expected cumulative timeout"))?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        Ok(())
    }
}
