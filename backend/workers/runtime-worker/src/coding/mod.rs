//! One worker owns task admission, source, coding, checking, recovery and delivery.
use crate::{
    operations::runtime::{checkout_with_deadline, git, validate_source},
    profiles::ayni::{attest, preflight},
};
use ayni_orchestrator_profile_ayni::coding::WorkerSpec;
use ayni_orchestrator_runtime::{
    activity::Stage,
    coding::{ExecutionOutcome, PublicationOutcome, WorkerPhase, WorkerResult},
    runtime_execution::read_bounded,
};
use sessions::{Context, SessionState};
use std::{fs, io, path::Path, sync::Arc};

pub mod activity;
mod commits;
mod diagnostics;
mod environment;
mod events;
pub mod model_usage;
mod publication;
mod quality;
mod recovery;
mod sessions;
mod tools;

/// Execute one frozen task, retaining separate execution, quality and delivery outcomes.
/// # Errors
/// Rejects malformed requests or inaccessible output; task outcomes are retained in result.json.
pub async fn run(
    spec: &WorkerSpec,
    output: &Path,
    activity: &activity::ActivityLog,
) -> io::Result<()> {
    validate_spec(spec)?;
    fs::create_dir_all(output)?;
    let context = Context {
        spec: Arc::new(spec.clone()),
        workspace: Path::new("/workspace").to_owned(),
        output: output.to_owned(),
        state: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        serial: Arc::new(tokio::sync::Mutex::new(())),
        activity: activity.clone(),
    };
    let reporter = events::Reporter::start(output, spec.reporting.as_ref())?;
    let mut result = empty_result();
    let operation = execute(&context, &mut result);
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let error = tokio::select! {
        value = operation => value.err(),
        _ = terminate.recv() => { result.execution = ExecutionOutcome::Cancelled; result.publication = if context.output.join("publication.json").exists() { PublicationOutcome::Failed } else { PublicationOutcome::NotRequired }; Some(io::Error::new(io::ErrorKind::Interrupted, "task cancelled")) },
        _ = tokio::signal::ctrl_c() => { result.execution = ExecutionOutcome::Cancelled; result.publication = if context.output.join("publication.json").exists() { PublicationOutcome::Failed } else { PublicationOutcome::NotRequired }; Some(io::Error::new(io::ErrorKind::Interrupted, "task cancelled")) },
    };
    #[cfg(not(unix))]
    let error = operation.await.err();
    if let Some(error) = error {
        context.activity.error(&error);
        let phase = context.state.lock().await.progress.phase.clone();
        if result.execution != ExecutionOutcome::Cancelled {
            result.execution = if error.kind() == io::ErrorKind::TimedOut {
                ExecutionOutcome::DeadlineExceeded
            } else if activity::unsafe_change(&error) {
                ExecutionOutcome::UnsafeChanges
            } else if matches!(phase, WorkerPhase::Admission | WorkerPhase::Checkout) {
                ExecutionOutcome::AdmissionFailed
            } else {
                ExecutionOutcome::Failed
            };
        }
    }
    let state = context.state.lock().await;
    result.coding_turns_consumed = state.progress.coding_turns_consumed;
    result.quality_round = state.progress.quality_round;
    drop(state);
    result.model_usage = collect_usage(&output.join("usage"))?;
    recovery::write_json(&output.join("result.json"), &result)?;
    context.state.lock().await.progress.publication = result.publication;
    context.phase(WorkerPhase::Completed).await?;
    if let Some(reporter) = reporter {
        reporter.finish().await?;
    }
    Ok(())
}

async fn execute(context: &Context, result: &mut WorkerResult) -> io::Result<()> {
    context.activity.record(Stage::RuntimeVerification);
    attest(&context.spec.execution)?;
    context.remaining()?;
    probe_sandbox(context).await?;
    let spec = context.spec.clone();
    let workspace = context.workspace.clone();
    let output = context.output.clone();
    let retained =
        tokio::task::spawn_blocking(move || recovery::restore(&spec, &workspace, &output))
            .await
            .map_err(io::Error::other)??;
    if let Some(state) = retained {
        *context.state.lock().await = state;
    }
    if context.spec.publication_only
        || context.output.join("final.json").exists()
        || context.output.join("publication.json").exists()
    {
        *result = recovery::read_json(&context.output.join("final.json"))?;
        validate_retained(context, result)?;
        return deliver_final(context, result).await;
    }
    prepare_workspace(context).await?;
    let coding_failed = code_source(context, result).await?;
    context.activity.record(Stage::Finalization);
    let spec = context.spec.clone();
    let workspace = context.workspace.clone();
    tokio::task::spawn_blocking(move || {
        commits::commit_changes(&spec, &workspace, "feat(task): implement requested changes")
    })
    .await
    .map_err(io::Error::other)??;
    context.save_source().await?;
    result.quality = quality::coordinate(context).await?;
    if result.requested_input.is_none() {
        result.requested_input = context
            .state
            .lock()
            .await
            .last_response
            .trim()
            .strip_prefix("INPUT_REQUIRED:")
            .map(|v| v.trim().chars().take(1024).collect::<String>())
            .filter(|v| !v.is_empty());
    }
    result.commit = Some(git(&context.workspace, &["rev-parse", "HEAD"])?);
    result.changed_paths = commits::changed_paths(&context.spec, &context.workspace)?;
    if result.requested_input.is_some() {
        result.execution = ExecutionOutcome::InputRequired;
        result.publication = PublicationOutcome::NotRequired;
        return Ok(());
    }
    if result.changed_paths.is_empty() {
        result.execution = ExecutionOutcome::NoChanges;
        result.publication = PublicationOutcome::NotRequired;
        return Ok(());
    }
    result.execution = if coding_failed {
        ExecutionOutcome::Failed
    } else {
        ExecutionOutcome::Succeeded
    };
    let progress = context.state.lock().await.progress.clone();
    result.coding_turns_consumed = progress.coding_turns_consumed;
    result.quality_round = progress.quality_round;
    context.save_source().await?;
    recovery::write_json(&context.output.join("final.json"), result)?;
    deliver_final(context, result).await
}
async fn code_source(context: &Context, result: &mut WorkerResult) -> io::Result<bool> {
    let mut coding_failed = false;
    if matches!(
        context.state.lock().await.progress.phase,
        WorkerPhase::Coding | WorkerPhase::Repairing
    ) && !context.state.lock().await.repair_stopped
        && context.coding_time().is_ok()
    {
        let progress = context.state.lock().await.progress.clone();
        let findings = if progress.phase == WorkerPhase::Repairing {
            progress
                .latest_full_check
                .as_ref()
                .map(tools::summary)
                .transpose()?
        } else {
            None
        };
        match sessions::run(context, findings.as_deref()).await {
            Ok(question) => {
                result.requested_input = question;
            }
            Err(error) => {
                context.activity.error(&error);
                coding_failed = true;
            }
        }
    }
    Ok(coding_failed)
}
async fn probe_sandbox(context: &Context) -> io::Result<()> {
    let probe = crate::operations::sandbox::command("/bin/true", Path::new("/"))?;
    if !crate::operations::process::status(
        &mut tokio::process::Command::from(probe),
        context.remaining()?.min(std::time::Duration::from_secs(10)),
    )
    .await?
    .success()
    {
        return Err(io::Error::other(
            "repository process sandbox is unavailable",
        ));
    }
    Ok(())
}
async fn deliver_final(context: &Context, result: &mut WorkerResult) -> io::Result<()> {
    context.phase(WorkerPhase::Publishing).await?;
    if let Err(error) = publication::deliver(context, result).await {
        context.activity.error(&error);
        if result.publication != PublicationOutcome::Conflict {
            result.publication = PublicationOutcome::Failed;
        }
    }
    recovery::write_json(&context.output.join("final.json"), result)
}
async fn prepare_workspace(context: &Context) -> io::Result<()> {
    if !context.workspace.join(".git").exists() {
        context.phase(WorkerPhase::Checkout).await?;
        context.activity.record(Stage::Checkout);
        let source_token = publication::token()?;
        checkout_with_deadline(
            &context.spec.execution,
            &context.workspace,
            Some(&source_token),
            context.remaining()?,
        )
        .await?;
        tracing::info!("frozen source checkout completed");
    }
    if context.spec.constraints.environment_change.is_some()
        && !matches!(
            context.state.lock().await.progress.phase,
            WorkerPhase::Admission | WorkerPhase::Checkout
        )
    {
        environment::validate_policy(context)?;
    } else {
        preflight(&context.spec.execution, &context.workspace)?;
    }
    tracing::info!("protected source inputs verified");
    let phase = context.state.lock().await.progress.phase.clone();
    if matches!(phase, WorkerPhase::Admission | WorkerPhase::Checkout) {
        // This focused invocation asks the supported CLI to verify the runtime's
        // certificate/protected content before exposing tools to the model.
        crate::profiles::ayni::invoke(
            &context.spec.execution,
            &context.workspace,
            &["verify".into(), "size".into()],
            false,
            context
                .coding_time()?
                .min(std::time::Duration::from_secs(180)),
        )
        .await?;
        context.phase(WorkerPhase::Coding).await?;
        context.save_source().await?;
    }
    Ok(())
}
fn validate_retained(context: &Context, result: &WorkerResult) -> io::Result<()> {
    environment::validate_retained(context)?;
    if result.commit.as_deref() != Some(git(&context.workspace, &["rev-parse", "HEAD"])?.as_str())
        || !git(&context.workspace, &["diff", "--name-only", "HEAD"])?.is_empty()
        || !git(
            &context.workspace,
            &["ls-files", "--others", "--exclude-standard"],
        )?
        .is_empty()
    {
        return Err(io::Error::other(
            "retained final workspace differs from final commit",
        ));
    }
    if let Some(evidence) = &result.quality.evidence {
        let bytes = read_bounded(
            &context.output.join("evidence/signals.json"),
            ayni_orchestrator_profile_ayni::managed_runtime::MAX_SIGNALS_BYTES,
        )?;
        if ayni_orchestrator_domain::digest::sha256(&bytes) != evidence.signals_digest
            || result.commit.as_deref() != Some(evidence.commit.as_str())
        {
            return Err(io::Error::other("retained final evidence is corrupt"));
        }
    } else if result.quality.outcome == ayni_orchestrator_runtime::coding::QualityOutcome::Passed {
        return Err(io::Error::other(
            "passing retained work requires full evidence",
        ));
    }
    Ok(())
}
fn empty_result() -> WorkerResult {
    WorkerResult {
        execution: ExecutionOutcome::Failed,
        commit: None,
        changed_paths: vec![],
        quality: quality::unavailable("Full checking has not completed"),
        model_usage: serde_json::Value::Null,
        requested_input: None,
        publication: PublicationOutcome::NotRequired,
        pull_request: None,
        coding_turns_consumed: 0,
        quality_round: 0,
    }
}
fn validate_spec(spec: &WorkerSpec) -> io::Result<()> {
    if let Some(profile) = &spec.profile {
        profile.validate(
            &ayni_orchestrator_runtime::agent_profile::Role::Worker,
            &spec.model,
            &profile.repository_ids,
        )?;
        if u32::from(spec.limits.coding_turns) > profile.settings.max_model_calls {
            return Err(io::Error::other("profile budget exceeded"));
        }
    }
    validate_source(&spec.execution)?;
    spec.constraints.validate().map_err(io::Error::other)?;
    let safe_identity = |v: &str| {
        !v.is_empty()
            && v.len() <= 128
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_'))
    };
    let safe_branch = |v: &str| {
        !v.is_empty()
            && v.len() <= 256
            && !v.starts_with('-')
            && !v.contains("..")
            && !v.ends_with('/')
            && !Path::new(v)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lock"))
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
    };
    if !safe_identity(&spec.task_id)
        || !safe_identity(&spec.attempt_id)
        || spec.request.is_empty()
        || spec.request.len() > 16384
        || spec.acceptance_criteria.is_empty()
        || spec.acceptance_criteria.len() > 20
        || spec
            .acceptance_criteria
            .iter()
            .map(String::len)
            .sum::<usize>()
            > 16384
        || spec.model.is_empty()
        || spec.model.len() > 128
        || spec.limits.coding_turns == 0
        || spec.limits.quality_repair_limit > 10
        || !ayni_orchestrator_domain::digest::digest(&spec.runtime_digest)
        || spec.execution.repository_url
            != format!("https://github.com/{}.git", spec.publication.repository)
        || !safe_branch(&spec.publication.branch)
        || !safe_branch(&spec.publication.base_branch)
    {
        return Err(io::Error::other("invalid frozen worker specification"));
    }
    Ok(())
}
fn collect_usage(directory: &Path) -> io::Result<serde_json::Value> {
    if !directory.exists() {
        return Ok(serde_json::Value::Null);
    }
    let mut reports = Vec::new();
    for entry in fs::read_dir(directory)?.take(64) {
        let entry = entry?;
        if entry.path().extension().is_some_and(|v| v == "json") {
            reports.push(serde_json::from_slice::<serde_json::Value>(&read_bounded(
                &entry.path(),
                4096,
            )?)?);
        }
    }
    Ok(serde_json::Value::Array(reports))
}

#[cfg(test)]
mod tests;

mod agent_runner;

mod report;

#[cfg(test)]
mod profile_tests;
