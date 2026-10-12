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
use tokio::io::{AsyncRead, AsyncReadExt};

/// Attest the immutable runtime metadata before source checkout.
/// # Errors
/// Rejects unsigned or mismatched runtime identities.
pub fn attest(request: &ExecutionRequest) -> io::Result<()> {
    validate_source(request)?;
    let metadata = read_bounded(
        Path::new("/etc/ayni/runtime.json"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    managed_runtime::signed_identity(&metadata, &request.expected).map_err(verification_error)?;
    Ok(())
}

fn verification_error(error: managed_runtime::RuntimeVerificationError) -> io::Error {
    use managed_runtime::RuntimeVerificationError as Error;
    io::Error::other(match error {
        Error::LockMismatch => "runtime_lock_mismatch",
        Error::PlatformMismatch => "runtime_platform_mismatch",
        Error::UnsupportedVersion => "runtime_version_unsupported",
        Error::Unsigned | Error::Malformed => "runtime_certificate_invalid",
        Error::MissingVerification => "runtime_verification_failed",
    })
}

/// Verify checkout and runtime identity before exposing coding tools.
/// # Errors
/// Rejects a changed source revision or missing protected policy inputs.
#[tracing::instrument(
    name = "verification.artifact_metadata",
    target = "ayni_orchestrator_observability::verification",
    skip_all
)]
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
    invoke_report(request, workspace, arguments, full, deadline, false)
        .await
        .map(|(report, _)| report)
}

pub(crate) async fn invoke_markdown(
    request: &ExecutionRequest,
    workspace: &Path,
    deadline: Duration,
) -> io::Result<(VerificationReport, Option<String>)> {
    invoke_report(request, workspace, &["check".into()], true, deadline, true).await
}

async fn invoke_report(
    request: &ExecutionRequest,
    workspace: &Path,
    arguments: &[String],
    full: bool,
    deadline: Duration,
    markdown: bool,
) -> io::Result<(VerificationReport, Option<String>)> {
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
    let (status, markdown) = execute(workspace, arguments, deadline, markdown).await?;
    let bytes = read_bounded(&signals, managed_runtime::MAX_SIGNALS_BYTES)?;
    let runtime = managed_runtime::verified_result(&metadata, &bytes, &request.expected)
        .map_err(verification_error)?;
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
    let mut quality = summarize(
        &bytes,
        status.success(),
        full,
        expected_command,
        &git(workspace, &["rev-parse", "HEAD"])?,
    )?;
    if let Some(markdown) = &markdown {
        report::decorate(&mut quality.rows, &artifact, markdown);
    }
    Ok((
        VerificationReport {
            revision: request.revision.clone(),
            lock_digest: format!("sha256:{:x}", Sha256::digest(lock)),
            runtime,
            check_exit_code: status.code().unwrap_or(-1),
            quality,
        },
        markdown,
    ))
}

#[tracing::instrument(
    name = "verification.ayni",
    target = "ayni_orchestrator_observability::verification",
    skip_all
)]
async fn execute(
    workspace: &Path,
    arguments: &[String],
    deadline: Duration,
    markdown: bool,
) -> io::Result<(std::process::ExitStatus, Option<String>)> {
    let mut process = crate::operations::sandbox::command("/usr/local/bin/ayni", workspace)?;
    process.args(arguments);
    if markdown {
        process.args(["--output", "markdown"]);
        let (status, output) = crate::operations::process::output(
            &mut tokio::process::Command::from(process),
            deadline,
            1_048_576,
        )
        .await?;
        Ok((
            status,
            Some(String::from_utf8(output).map_err(io::Error::other)?),
        ))
    } else {
        Ok((
            crate::operations::process::status(
                &mut tokio::process::Command::from(process),
                deadline,
            )
            .await?,
            None,
        ))
    }
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

#[tracing::instrument(
    name = "verification.artifact_validation",
    target = "ayni_orchestrator_observability::verification",
    skip_all
)]
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
                    status: String::new(),
                    summary: String::new(),
                    target: String::new(),
                    passed: row["pass"]
                        .as_bool()
                        .ok_or_else(|| io::Error::other("invalid row outcome"))?,
                })
            })
            .collect::<io::Result<_>>()?,
        findings,
    })
}

/// Build and certify through a separately approved isolated TLS engine.
/// Never invoked by a coding Job or the controller.
///
/// # Errors
/// Rejects missing signing material, source mismatch, or failed build/certification.
pub async fn build(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    validate_source(request)?;
    validate_cli_version(&request.expected.ayni_version, workspace).await?;
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
    let image = build_target()?;
    let lock: Value = serde_json::from_slice(&read_bounded(
        &workspace.join(".ayni.lock"),
        managed_runtime::MAX_METADATA_BYTES,
    )?)
    .map_err(io::Error::other)?;
    if lock["fingerprint"] != request.expected.lock_fingerprint
        || lock["ayni_version"] != request.expected.ayni_version
    {
        return Err(io::Error::other("runtime_lock_mismatch"));
    }
    let builder = std::env::var("AYNI_BUILDER_IMAGE").map_err(io::Error::other)?;
    ayni_orchestrator_profile_ayni::compatibility::validate_builder(&builder)
        .map_err(io::Error::other)?;
    ayni_orchestrator_profile_ayni::compatibility::validate_lock_envelope(
        &serde_json::to_vec(&lock).map_err(io::Error::other)?,
        &request.expected.platform,
    )
    .map_err(io::Error::other)?;
    wait_engine(workspace).await?;
    let values = build_environment(true)?;
    let mut build = command("/usr/local/bin/ayni", workspace);
    build
        .envs(values.iter().map(|(k, v)| (*k, v)))
        .env("AYNI_OCI_RUNTIME", "docker")
        .args([
            "env",
            "build",
            "--executor-image",
            &builder,
            "--tag",
            &image,
        ]);
    success(build)
        .await
        .map_err(|_| io::Error::other("runtime_build_failed"))?;
    let mut inspect = command("docker", workspace);
    inspect
        .envs(
            values
                .iter()
                .filter(|(k, _)| k.starts_with("DOCKER_"))
                .map(|(k, v)| (*k, v)),
        )
        .args([
            "image",
            "inspect",
            "--format",
            "{{.Id}} {{.Os}}/{{.Architecture}}",
            &image,
        ]);
    let output = capture(inspect).await?;
    let (id, platform) = output
        .trim()
        .split_once(' ')
        .ok_or_else(|| io::Error::other("runtime_build_result_invalid"))?;
    if !image_id(id) || platform != request.expected.platform {
        return Err(io::Error::other("runtime_platform_mismatch"));
    }
    if crate::operation::enabled() {
        return crate::operation::complete(
            ayni_orchestrator_runtime::operation::OperationOutcome::Built {
                candidate_image_id: id.into(),
            },
        )
        .await;
    }
    fs::write("/dev/termination-log", id)
}

async fn validate_cli_version(version: &str, workspace: &Path) -> io::Result<()> {
    ayni_orchestrator_profile_ayni::compatibility::contract(version).map_err(io::Error::other)?;
    let mut cli = command("/usr/local/bin/ayni", workspace);
    cli.arg("--version");
    if capture(cli).await?.trim() != format!("ayni {version}") {
        return Err(io::Error::other("runtime_version_unsupported"));
    }
    Ok(())
}

pub(crate) fn build_target() -> io::Result<String> {
    let image = std::env::var("BUILD_IMAGE").map_err(io::Error::other)?;
    if image.is_empty()
        || image.starts_with('-')
        || image.contains('@')
        || image.len() > 512
        || image.contains(['\n', '\r'])
    {
        return Err(io::Error::other("runtime_image_invalid"));
    }
    Ok(image)
}

pub(crate) fn image_id(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

pub(crate) fn build_environment(signing: bool) -> io::Result<Vec<(&'static str, String)>> {
    let mut variables = vec![
        "DOCKER_HOST",
        "DOCKER_TLS_VERIFY",
        "DOCKER_CERT_PATH",
        "DOCKER_CONFIG",
    ];
    if signing {
        variables.extend([
            "AYNI_ENV_CERTIFICATE_SIGNING_KEY",
            "AYNI_ENV_CERTIFICATE_KEY_ID",
        ]);
    }
    variables
        .into_iter()
        .map(|name| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|value| (name, value))
                .ok_or_else(|| io::Error::other("runtime_build_credentials_missing"))
        })
        .collect()
}

/// Publish the exact certified candidate without signing authority or repository execution.
/// # Errors
/// Rejects unavailable candidates, malformed inputs or failed publication.
pub async fn publish(
    request: &ayni_orchestrator_runtime::image_build::ImagePublicationRequest,
    workspace: &Path,
) -> io::Result<()> {
    let image = build_target()?;
    let id = std::env::var("BUILD_IMAGE_ID").map_err(io::Error::other)?;
    if !image_id(&id) {
        return Err(io::Error::other("runtime_build_result_invalid"));
    }
    let values = build_environment(false)?;
    let overwrite_authorized = std::env::var("AYNI_REBUILD_AUTHORIZED").as_deref() == Ok("true");
    let existing = match crate::operations::image_discovery::inspect(&image, &request.platform)
        .await
    {
        Ok(existing) => existing,
        Err("runtime_platform_mismatch" | "runtime_digest_mismatch") if overwrite_authorized => {
            ayni_orchestrator_runtime::image_discovery::ImageDiscovery::Missing
        }
        Err(reason) => return Err(io::Error::other(reason)),
    };
    publication_guard(&existing, &id, overwrite_authorized)?;
    let mut inspect = command("docker", workspace);
    inspect
        .envs(values.iter().map(|(k, v)| (*k, v)))
        .args(["image", "inspect", "--format", "{{.Id}}", &id]);
    if capture(inspect)
        .await
        .map_err(|_| io::Error::other("runtime_candidate_missing"))?
        .trim()
        != id
    {
        return Err(io::Error::other("runtime_candidate_missing"));
    }
    let mut tag = command("docker", workspace);
    tag.envs(values.iter().map(|(k, v)| (*k, v)))
        .args(["tag", &id, &image]);
    success(tag)
        .await
        .map_err(|_| io::Error::other("runtime_publication_failed"))?;
    let mut push = command("docker", workspace);
    push.stderr(std::process::Stdio::inherit());
    push.envs(values.iter().map(|(k, v)| (*k, v)))
        .args(["push", &image]);
    success(push)
        .await
        .map_err(|_| io::Error::other("runtime_publication_failed"))?;
    if crate::operation::enabled() {
        return crate::operation::complete(
            ayni_orchestrator_runtime::operation::OperationOutcome::Published,
        )
        .await;
    }
    fs::write("/dev/termination-log", "published")
}

fn publication_guard(
    existing: &ayni_orchestrator_runtime::image_discovery::ImageDiscovery,
    candidate: &str,
    overwrite_authorized: bool,
) -> io::Result<()> {
    use ayni_orchestrator_runtime::image_discovery::ImageDiscovery;
    match existing {
        ImageDiscovery::Missing => Ok(()),
        ImageDiscovery::Resolved { image_id, .. }
            if image_id == candidate || overwrite_authorized =>
        {
            Ok(())
        }
        ImageDiscovery::Resolved { .. } => Err(io::Error::other("runtime_tag_drift")),
        ImageDiscovery::Failed { .. } | ImageDiscovery::Pending => {
            Err(io::Error::other("runtime_publication_failed"))
        }
    }
}

async fn bounded_output<R: AsyncRead + Unpin>(reader: R) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(4097).read_to_end(&mut bytes).await?;
    if bytes.len() > 4096 {
        return Err(io::Error::other("runtime_build_result_invalid"));
    }
    Ok(bytes)
}

pub(crate) async fn capture(command: std::process::Command) -> io::Result<String> {
    let mut command = tokio::process::Command::from(command);
    command.stdout(std::process::Stdio::piped());
    let mut child = crate::operations::process::spawn(&mut command)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("runtime_build_result_invalid"))?;
    let (status, bytes) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(child.wait(), bounded_output(stdout))
    })
    .await
    .map_err(|_| io::Error::other("runtime_build_timeout"))?;
    if !status?.success() {
        return Err(io::Error::other("runtime_build_failed"));
    }
    String::from_utf8(bytes?).map_err(io::Error::other)
}

/// Wait for attempt-local mutual TLS without consuming the build execution deadline.
pub(crate) async fn wait_engine(workspace: &Path) -> io::Result<()> {
    let values = build_environment(false)?;
    for _ in 0..60 {
        let mut probe = command("docker", workspace);
        probe
            .args(["info"])
            .envs(values.iter().map(|(k, v)| (*k, v)));
        if crate::operations::process::status(
            &mut tokio::process::Command::from(probe),
            Duration::from_secs(3),
        )
        .await
        .is_ok_and(|status| status.success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(io::Error::other("engine_tls_or_startup_failed"))
}

pub(crate) async fn success(command: std::process::Command) -> io::Result<()> {
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

mod report;
