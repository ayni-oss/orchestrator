//! Supported Ayni invocation, provenance admission and environment builds.
use crate::operations::runtime::{command, git, validate_source};
use ayni_orchestrator_profile_ayni::{
    managed_runtime,
    runtime_execution::{ExecutionRequest, VerificationReport},
};
use ayni_orchestrator_runtime::{
    coding::{QualityEvidence, QualityOutcome, QualityResult, QualityRow},
    runtime_execution::{EXECUTION_DEADLINE_SECONDS, read_bounded},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path, time::Duration};

/// Attest the immutable runtime metadata before source checkout.
/// # Errors
/// Rejects unsigned or mismatched runtime identities.
pub fn attest(request: &ExecutionRequest) -> io::Result<()> {
    validate_source(request)?;
    let metadata = read_bounded(
        Path::new("/etc/ayni/runtime.json"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    managed_runtime::signed_identity(&metadata, &request.expected).map_err(io::Error::other)?;
    Ok(())
}

/// Verify checkout and runtime identity before exposing coding tools.
/// # Errors
/// Rejects a changed source revision or missing protected policy inputs.
pub fn preflight(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    attest(request)?;
    git(
        workspace,
        &["merge-base", "--is-ancestor", &request.revision, "HEAD"],
    )?;
    for path in [".ayni.lock", ".ayni.toml"] {
        let expected = git(
            workspace,
            &["show", &format!("{}:{path}", request.revision)],
        )?;
        let actual = read_bounded(&workspace.join(path), managed_runtime::MAX_METADATA_BYTES)?;
        if actual.strip_suffix(b"\n").unwrap_or(&actual) != expected.as_bytes() {
            return Err(io::Error::other("protected runtime inputs changed"));
        }
    }
    Ok(())
}

/// Run the complete repository contract with a fresh full artifact.
/// # Errors
/// Fails on unavailable execution, invalid evidence or mismatched provenance.
pub async fn check(request: &ExecutionRequest, workspace: &Path) -> io::Result<VerificationReport> {
    invoke(
        request,
        workspace,
        &["check".into()],
        true,
        Duration::from_secs(EXECUTION_DEADLINE_SECONDS),
    )
    .await
}

/// Run a focused signal; its evidence never grants full repository completion.
/// # Errors
/// Returns Ayni selector, execution and provenance errors explicitly.
pub async fn invoke(
    request: &ExecutionRequest,
    workspace: &Path,
    arguments: &[String],
    full: bool,
    deadline: Duration,
) -> io::Result<VerificationReport> {
    let expected_command = artifact_command(arguments, full)?;
    preflight(request, workspace)?;
    let lock = read_bounded(
        &workspace.join(".ayni.lock"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    let policy = read_bounded(
        &workspace.join(".ayni.toml"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    let metadata = read_bounded(
        Path::new("/etc/ayni/runtime.json"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    let directory = if full {
        workspace.join(".ayni/last")
    } else {
        workspace.join(".ayni/verify/last")
    };
    for path in [
        workspace.join(".ayni"),
        directory.clone(),
        directory
            .parent()
            .ok_or_else(|| io::Error::other("artifact parent missing"))?
            .to_owned(),
    ] {
        if path.exists() && !fs::symlink_metadata(&path)?.is_dir() {
            return Err(io::Error::other("unsafe Ayni output directory"));
        }
    }
    let signals = directory.join("signals.json");
    match fs::remove_file(&signals) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let started = std::time::Instant::now();
    let mut process = crate::operations::sandbox::command("/usr/local/bin/ayni", workspace)?;
    process.args(arguments);
    let status =
        crate::operations::process::status(&mut tokio::process::Command::from(process), deadline)
            .await?;
    let bytes = read_bounded(&signals, managed_runtime::MAX_SIGNALS_BYTES)?;
    let runtime = managed_runtime::verified_result(&metadata, &bytes, &request.expected)
        .map_err(io::Error::other)?;
    // Delegate full schema/finding/canonical-summary validation to the pinned CLI,
    // rather than duplicating Ayni's adapter or wire validation rules.
    let mut parse = command("/usr/local/bin/ayni", workspace);
    parse.args(["verify", "list", "--artifact"]).arg(&signals);
    if !crate::operations::process::status(
        &mut tokio::process::Command::from(parse),
        deadline.saturating_sub(started.elapsed()),
    )
    .await?
    .success()
    {
        return Err(io::Error::other("Ayni rejected its emitted artifact"));
    }
    let artifact: Value = serde_json::from_slice(&bytes)?;
    if artifact["contract_digest"] != format!("sha256:{:x}", Sha256::digest(&policy)) {
        return Err(io::Error::other(
            "artifact contract digest does not match frozen policy",
        ));
    }
    if read_bounded(
        &workspace.join(".ayni.lock"),
        managed_runtime::MAX_METADATA_BYTES,
    )? != lock
        || read_bounded(
            &workspace.join(".ayni.toml"),
            managed_runtime::MAX_METADATA_BYTES,
        )? != policy
    {
        return Err(io::Error::other(
            "verification changed protected source inputs",
        ));
    }
    let quality = summarize(
        &bytes,
        status.success(),
        full,
        expected_command,
        &git(workspace, &["rev-parse", "HEAD"])?,
    )?;
    Ok(VerificationReport {
        revision: request.revision.clone(),
        lock_digest: format!("sha256:{:x}", Sha256::digest(lock)),
        runtime,
        check_exit_code: status.code().unwrap_or(-1),
        quality,
    })
}

fn required_count(value: &Value, section: &str, field: &str) -> io::Result<u32> {
    value[section][field]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| io::Error::other("invalid quality measurement"))
}

fn artifact_command(arguments: &[String], full: bool) -> io::Result<&'static str> {
    let command = arguments.first().map(String::as_str);
    let selector = arguments.get(1).map(String::as_str);
    match (full, command, selector) {
        (true, Some("check"), None) => Ok("check"),
        (false, Some("verify"), Some("test")) => Ok("verify_test"),
        (false, Some("verify"), Some("coverage")) => Ok("verify_coverage"),
        (false, Some("verify"), Some("size")) => Ok("verify_size"),
        (false, Some("verify"), Some("complexity")) => Ok("verify_complexity"),
        (false, Some("verify"), Some("deps")) => Ok("verify_deps"),
        (false, Some("verify"), Some("mutation")) => Ok("verify_mutation"),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported Ayni verification invocation",
        )),
    }
}

fn summarize(
    bytes: &[u8],
    success: bool,
    full: bool,
    expected_command: &str,
    commit: &str,
) -> io::Result<QualityResult> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value["invocation"]["command"] != expected_command
        || value["completion"]["scope"] != if full { "repository" } else { "requested" }
    {
        return Err(io::Error::other("Ayni artifact scope mismatch"));
    }
    let fingerprint = value["source_fingerprint"]
        .as_str()
        .filter(|v| ayni_orchestrator_domain::digest::digest(v))
        .ok_or_else(|| io::Error::other("source fingerprint missing"))?;
    let rows = value["rows"]
        .as_array()
        .ok_or_else(|| io::Error::other("quality rows missing"))?;
    let mut findings = Vec::new();
    for row in rows {
        for finding in row["offenders"]["items"].as_array().into_iter().flatten() {
            if findings.len() == 32 {
                break;
            }
            let content = serde_json::to_string(finding)?;
            findings.push(content.chars().take(1024).collect());
        }
        if let Some(failure) = row["result"]["failure"].as_object()
            && findings.len() < 32
        {
            findings.push(serde_json::to_string(failure)?.chars().take(1024).collect());
        }
    }
    for issue in value["completion"]["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .take(32_usize.saturating_sub(findings.len()))
    {
        findings.push(serde_json::to_string(issue)?.chars().take(1024).collect());
    }
    let passed = success
        && value["aggregate"]["status"] == "pass"
        && value["completion"]["state"] == "complete"
        && !rows.is_empty();
    // Operational command failures are unavailable checks, not source defects.
    let actionable_command_failure = |row: &Value| {
        let failure = &row["result"]["failure"];
        failure["classification"] == "command_error"
            && matches!(
                failure["category"].as_str(),
                Some("repo_code_issue" | "repo_setup_issue")
            )
            && failure["exit_code"].as_i64().is_some_and(|code| code != 0)
    };
    let operational = rows
        .iter()
        .any(|row| row["result"]["failure"].is_object() && !actionable_command_failure(row))
        || (value["completion"]["state"] != "complete"
            && !rows.iter().any(actionable_command_failure));
    Ok(QualityResult {
        outcome: if passed {
            QualityOutcome::Passed
        } else if operational {
            QualityOutcome::Unavailable
        } else {
            QualityOutcome::Failed
        },
        evidence: full.then(|| QualityEvidence {
            artifact_path: "evidence/signals.json".into(),
            signals_digest: format!("sha256:{:x}", Sha256::digest(bytes)),
            source_fingerprint: fingerprint.into(),
            commit: commit.into(),
        }),
        completed_targets: Some(required_count(&value, "completion", "completed_targets")?),
        skipped_targets: Some(required_count(&value, "completion", "skipped_targets")?),
        warning_offenders: Some(required_count(&value, "aggregate", "warning_offenders")?),
        failing_offenders: Some(required_count(&value, "aggregate", "failing_offenders")?),
        rows: rows
            .iter()
            .take(32)
            .map(|row| {
                Ok(QualityRow {
                    kind: row["kind"]
                        .as_str()
                        .ok_or_else(|| io::Error::other("invalid row kind"))?
                        .into(),
                    passed: row["pass"]
                        .as_bool()
                        .ok_or_else(|| io::Error::other("invalid row outcome"))?,
                })
            })
            .collect::<io::Result<_>>()?,
        findings,
    })
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

#[cfg(test)]
mod tests;

pub mod trust_lock;
