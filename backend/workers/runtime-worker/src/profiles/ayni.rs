//! Ayni verification and build invocation inside authorized worker Jobs.
use crate::operations::runtime::{command, git, validate_source};
use ayni_orchestrator_profile_ayni::{
    managed_runtime,
    runtime_execution::{ExecutionRequest, VerificationReport},
};
use ayni_orchestrator_runtime::{
    coding::{ValidationOutcome, ValidationResult, ValidationRow},
    runtime_execution::{EXECUTION_DEADLINE_SECONDS, read_bounded},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};

/// Run Ayni verification in the signed runtime before coding. Old signals are
/// removed, lock/policy changes are rejected, and Ayni owns cryptographic checks.
///
/// # Errors
/// Returns an error when source, certificate, or freshly emitted provenance differs.
pub async fn verify(
    request: &ExecutionRequest,
    workspace: &Path,
) -> io::Result<VerificationReport> {
    let request = request.clone();
    let workspace = workspace.to_owned();
    let inputs = tokio::task::spawn_blocking(move || prepare(request, workspace))
        .await
        .map_err(io::Error::other)??;
    let mut check = command("/usr/local/bin/ayni", &inputs.workspace);
    check.arg("check");
    let status = crate::operations::process::status(
        &mut tokio::process::Command::from(check),
        Duration::from_secs(EXECUTION_DEADLINE_SECONDS),
    )
    .await?;
    tokio::task::spawn_blocking(move || finish(inputs, status))
        .await
        .map_err(io::Error::other)?
}

/// Verify immutable runtime admission before model tools are exposed. Quality
/// validation is intentionally performed only by the coding tools.
pub fn preflight(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    validate_source(request)?;
    if git(
        workspace,
        &["merge-base", "--is-ancestor", &request.revision, "HEAD"],
    )
    .is_err()
    {
        return Err(io::Error::other("source revision mismatch"));
    }
    let metadata = read_bounded(
        Path::new("/etc/ayni/runtime.json"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    managed_runtime::signed_identity(&metadata, &request.expected).map_err(io::Error::other)?;
    for path in [workspace.join(".ayni.lock"), workspace.join(".ayni.toml")] {
        read_bounded(&path, managed_runtime::MAX_METADATA_BYTES)?;
    }
    Ok(())
}

struct VerificationInputs {
    request: ExecutionRequest,
    workspace: PathBuf,
    lock: Vec<u8>,
    policy: Vec<u8>,
    metadata: Vec<u8>,
    signals: PathBuf,
}

fn prepare(request: ExecutionRequest, workspace: PathBuf) -> io::Result<VerificationInputs> {
    validate_source(&request)?;
    if git(
        &workspace,
        &["merge-base", "--is-ancestor", &request.revision, "HEAD"],
    )
    .is_err()
    {
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
    Ok(VerificationInputs {
        request,
        workspace,
        lock,
        policy,
        metadata,
        signals,
    })
}

fn finish(inputs: VerificationInputs, status: ExitStatus) -> io::Result<VerificationReport> {
    let VerificationInputs {
        request,
        workspace,
        lock,
        policy,
        metadata,
        signals,
    } = inputs;
    let lock_path = workspace.join(".ayni.lock");
    let policy_path = workspace.join(".ayni.toml");
    let bytes = read_bounded(&signals, managed_runtime::MAX_SIGNALS_BYTES)?;
    let runtime = managed_runtime::verified_result(&metadata, &bytes, &request.expected)
        .map_err(io::Error::other)?;
    if read_bounded(&lock_path, managed_runtime::MAX_METADATA_BYTES)? != lock
        || read_bounded(&policy_path, managed_runtime::MAX_METADATA_BYTES)? != policy
        || git(
            &workspace,
            &["merge-base", "--is-ancestor", &request.revision, "HEAD"],
        )
        .is_err()
    {
        return Err(io::Error::other(
            "verification changed protected source inputs",
        ));
    }
    Ok(VerificationReport {
        revision: request.revision,
        lock_digest: format!("sha256:{:x}", Sha256::digest(lock)),
        runtime,
        check_exit_code: status.code().unwrap_or(-1),
        validation: validation(&bytes, status.success()),
    })
}

fn validation(bytes: &[u8], success: bool) -> ValidationResult {
    let parsed = serde_json::from_slice::<Value>(bytes).ok();
    let count = |section: &str, field: &str| {
        parsed
            .as_ref()
            .and_then(|value| value[section][field].as_u64())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    };
    let rows = parsed
        .as_ref()
        .and_then(|value| value["rows"].as_array())
        .into_iter()
        .flatten()
        .take(32)
        .filter_map(|row| {
            let kind = row["kind"].as_str()?;
            let passed = row["pass"].as_bool()?;
            (kind.len() <= 64).then(|| ValidationRow {
                kind: kind.into(),
                passed,
            })
        })
        .collect();
    ValidationResult {
        outcome: if success {
            ValidationOutcome::Passed
        } else {
            ValidationOutcome::Failed
        },
        signals_digest: Some(format!("sha256:{:x}", Sha256::digest(bytes))),
        completed_targets: count("completion", "completed_targets"),
        skipped_targets: count("completion", "skipped_targets"),
        warning_offenders: count("aggregate", "warning_offenders"),
        failing_offenders: count("aggregate", "failing_offenders"),
        rows,
    }
}

/// Build-only entrypoint for a separately authorized TLS engine and push Secret.
/// Never invoked by a coding Job or the controller.
///
/// # Errors
/// Rejects missing signing material, source mismatch, or failed build/publication.
pub async fn build(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    validate_source(request)?;
    let source = request.revision.clone();
    let directory = workspace.to_owned();
    tokio::task::spawn_blocking(move || {
        if git(&directory, &["rev-parse", "HEAD"])? != source {
            return Err(io::Error::other("build source mismatch"));
        }
        Ok(())
    })
    .await
    .map_err(io::Error::other)??;
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
    success(build).await?;
    let mut push = command("docker", workspace);
    push.envs(
        values
            .iter()
            .filter(|(k, _)| k.starts_with("DOCKER_"))
            .map(|(k, v)| (*k, v)),
    )
    .args(["push", &image]);
    success(push).await
}

async fn success(command: std::process::Command) -> io::Result<()> {
    if crate::operations::process::status(
        &mut tokio::process::Command::from(command),
        Duration::from_secs(EXECUTION_DEADLINE_SECONDS),
    )
    .await?
    .success()
    {
        Ok(())
    } else {
        Err(io::Error::other("runtime command failed"))
    }
}
