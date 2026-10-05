//! Ayni verification and build invocation inside authorized worker Jobs.
use crate::operations::runtime::{command, git, success, validate_source};
use ayni_orchestrator_profile_ayni::{
    managed_runtime,
    runtime_execution::{ExecutionRequest, VerificationReport},
};
use ayni_orchestrator_runtime::runtime_execution::read_bounded;
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};

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
