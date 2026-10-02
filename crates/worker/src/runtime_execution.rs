//! Trusted launch operations for a mounted checkout and an admitted runtime.

use crate::managed_runtime;
pub use ayni_orchestrator_runtime::runtime_execution::{
    ExecutionRequest, VerificationReport, read_bounded,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
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
pub fn checkout(
    request: &ExecutionRequest,
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
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
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

/// Run Ayni verification in the signed runtime before coding. Old signals are
/// removed, lock/policy changes are rejected, and Ayni owns cryptographic checks.
///
/// # Errors
/// Returns an error when source, certificate, or freshly emitted provenance differs.
pub fn verify(request: &ExecutionRequest, workspace: &Path) -> io::Result<VerificationReport> {
    validate_source(request)?;
    if git(workspace, &["rev-parse", "HEAD"])? != request.revision {
        return Err(io::Error::other("source revision mismatch"));
    }
    let lock_path = workspace.join(".ayni.lock");
    let policy_path = workspace.join(".ayni.toml");
    let lock = read_bounded(&lock_path, managed_runtime::MAX_METADATA_BYTES)?;
    let policy = read_bounded(&policy_path, managed_runtime::MAX_METADATA_BYTES)?;
    let metadata = read_bounded(
        Path::new("/etc/ayni/runtime.json"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    managed_runtime::signed_identity(&metadata, &request.expected).map_err(io::Error::other)?;
    let state = workspace.join(".ayni");
    for path in [&state, &state.join("last")] {
        if path.exists() && !fs::symlink_metadata(path)?.is_dir() {
            return Err(io::Error::other("unsafe Ayni output directory"));
        }
    }
    let signals = state.join("last/signals.json");
    match fs::remove_file(&signals) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let status = command("/usr/local/bin/ayni", workspace)
        .arg("check")
        .status()?;
    let bytes = read_bounded(&signals, managed_runtime::MAX_SIGNALS_BYTES)?;
    let runtime = managed_runtime::verified_result(&metadata, &bytes, &request.expected)
        .map_err(io::Error::other)?;
    if read_bounded(&lock_path, managed_runtime::MAX_METADATA_BYTES)? != lock
        || read_bounded(&policy_path, managed_runtime::MAX_METADATA_BYTES)? != policy
        || git(workspace, &["rev-parse", "HEAD"])? != request.revision
    {
        return Err(io::Error::other(
            "verification changed protected source inputs",
        ));
    }
    Ok(VerificationReport {
        revision: request.revision.clone(),
        lock_digest: format!("sha256:{:x}", Sha256::digest(lock)),
        runtime,
        check_exit_code: status.code().unwrap_or(-1),
    })
}

fn validate_source(request: &ExecutionRequest) -> io::Result<()> {
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

/// Build-only entrypoint for a separately authorized TLS engine and push Secret.
/// Never invoked by a coding Job or the controller.
///
/// # Errors
/// Rejects missing signing material, source mismatch, or failed build/publication.
pub fn build(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    validate_source(request)?;
    if git(workspace, &["rev-parse", "HEAD"])? != request.revision {
        return Err(io::Error::other("build source mismatch"));
    }
    let image = std::env::var("BUILD_IMAGE").map_err(io::Error::other)?;
    if image.is_empty() || image.starts_with('-') || image.contains('@') || image.len() > 512 {
        return Err(io::Error::other("invalid build target"));
    }
    let variables = [
        "DOCKER_HOST",
        "DOCKER_TLS_VERIFY",
        "DOCKER_CERT_PATH",
        "DOCKER_CONFIG",
        "AYNI_ENV_CERTIFICATE_SIGNING_KEY",
        "AYNI_ENV_CERTIFICATE_KEY_ID",
    ];
    let values: Vec<_> = variables
        .iter()
        .map(|name| {
            std::env::var(name)
                .map(|value| (*name, value))
                .map_err(io::Error::other)
        })
        .collect::<Result<_, _>>()?;
    if values.iter().any(|(_, v)| v.is_empty()) {
        return Err(io::Error::other("build credentials missing"));
    }
    let mut build = command("/usr/local/bin/ayni", workspace);
    build
        .envs(values.iter().map(|(k, v)| (*k, v)))
        .env("AYNI_OCI_RUNTIME", "docker")
        .args(["env", "build", "--tag", &image]);
    success(&mut build)?;
    let mut push = command("docker", workspace);
    push.envs(
        values
            .iter()
            .filter(|(k, _)| k.starts_with("DOCKER_"))
            .map(|(k, v)| (*k, v)),
    )
    .args(["push", &image]);
    success(&mut push)
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
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn success(command: &mut Command) -> io::Result<()> {
    if command.status()?.success() {
        Ok(())
    } else {
        Err(io::Error::other("runtime command failed"))
    }
}

pub(crate) fn git(workspace: &Path, arguments: &[&str]) -> io::Result<String> {
    let mut child = command("git", workspace)
        .args(arguments)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("git stdout missing"))?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("git output oversized"));
    }
    if !child.wait()?.success() {
        return Err(io::Error::other("source operation failed"));
    }
    String::from_utf8(bytes)
        .map(|value| value.trim_end_matches('\n').to_owned())
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_runtime::RuntimeExpectation;
    #[test]
    fn rejects_untrusted_checkout_transport_and_revision() {
        let mut request = ExecutionRequest {
            repository_url: "https://github.com/team/repo.git".into(),
            revision: "a".repeat(40),
            expected: RuntimeExpectation {
                lock_fingerprint: String::new(),
                platform: String::new(),
                ayni_version: String::new(),
            },
        };
        assert!(validate_source(&request).is_ok());
        request.repository_url = "https://github.com@attacker.test/team/repo.git".into();
        assert!(validate_source(&request).is_err());
        request.repository_url = "https://github.com/team/repo.git".into();
        request.revision = "main".into();
        assert!(validate_source(&request).is_err());
    }
}
