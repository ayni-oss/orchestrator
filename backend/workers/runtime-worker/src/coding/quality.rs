//! Mandatory full checking, operational retries and commit/evidence binding.
use super::{
    commits, recovery,
    sessions::{Context, PUBLICATION_RESERVE_SECONDS},
};
use ayni_orchestrator_runtime::{
    activity::Stage,
    coding::{QualityOutcome, QualityResult, WorkerPhase},
    runtime_execution::read_bounded,
};
use std::{io, time::Duration};

pub fn unavailable(message: &str) -> QualityResult {
    QualityResult {
        outcome: QualityOutcome::Unavailable,
        evidence: None,
        completed_targets: None,
        skipped_targets: None,
        warning_offenders: None,
        failing_offenders: None,
        rows: vec![],
        findings: vec![message.into()],
    }
}
/// Only the supported CLI invocation and coding session vary; coordination,
/// commits, recovery and evidence freezing use the same implementation in tests.
trait Runner: Sync {
    fn check(
        &self,
        context: &Context,
        deadline: Duration,
    ) -> impl std::future::Future<Output = io::Result<QualityResult>> + Send;
    fn repair(
        &self,
        context: &Context,
        findings: &str,
    ) -> impl std::future::Future<Output = io::Result<Option<String>>> + Send;
}
struct NativeRunner;
impl Runner for NativeRunner {
    async fn check(&self, context: &Context, deadline: Duration) -> io::Result<QualityResult> {
        if let Some(unavailable) = super::environment::prepare(context, deadline).await? {
            return Ok(unavailable);
        }
        let (report, markdown) = crate::profiles::ayni::invoke_markdown(
            &context.spec.execution,
            &context.workspace,
            deadline,
        )
        .await?;
        let path = context.output.join("quality-report.pending.json");
        if let (Some(evidence), Some(markdown)) = (&report.quality.evidence, markdown) {
            recovery::write_json(
                &path,
                &super::report::Report {
                    signals_digest: evidence.signals_digest.clone(),
                    commit: evidence.commit.clone(),
                    markdown,
                },
            )?;
        } else if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(report.quality)
    }
    async fn repair(&self, context: &Context, findings: &str) -> io::Result<Option<String>> {
        super::sessions::run(context, Some(findings)).await
    }
}

async fn full(context: &Context, runner: &impl Runner) -> io::Result<QualityResult> {
    let mut latest = unavailable("Full repository checking is unavailable");
    for _ in 0..3 {
        let time = context
            .remaining()?
            .saturating_sub(Duration::from_secs(PUBLICATION_RESERVE_SECONDS))
            .min(Duration::from_secs(1800));
        if time.is_zero() {
            return Ok(unavailable(
                "Deadline reserves remaining time for publication",
            ));
        }
        match runner.check(context, time).await {
            Ok(result) => {
                latest = result;
            }
            Err(error)
                if error.to_string().contains("protected")
                    || error.to_string().contains("source revision") =>
            {
                return Err(error);
            }
            Err(_) => {
                latest = unavailable(
                    "Full check execution or evidence validation failed; measurements are unavailable",
                );
            }
        }
        if latest.outcome != QualityOutcome::Unavailable {
            break;
        }
    }
    Ok(latest)
}

pub async fn coordinate(context: &Context) -> io::Result<QualityResult> {
    coordinate_with(context, &NativeRunner).await
}

async fn coordinate_with(context: &Context, runner: &impl Runner) -> io::Result<QualityResult> {
    loop {
        context.phase(WorkerPhase::Checking).await?;
        let mut latest = full(context, runner).await?;
        // A collector that changes source invalidates the first artifact. Commit
        // actual permitted changes and check the resulting exact commit again.
        for index in 0..3 {
            let spec = context.spec.clone();
            let workspace = context.workspace.clone();
            let changed = tokio::task::spawn_blocking(move || {
                commits::commit_changes(
                    &spec,
                    &workspace,
                    "chore(task): retain changes produced by quality tools",
                )
            })
            .await
            .map_err(io::Error::other)??;
            if changed.is_none() {
                break;
            }
            context.save_source().await?;
            if index == 2 {
                latest = unavailable(
                    "Quality tools repeatedly changed source; final evidence cannot be bound",
                );
                break;
            }
            latest = full(context, runner).await?;
        }
        context.state.lock().await.progress.latest_full_check = Some(latest.clone());
        context.persist().await?;
        context.save_source().await?;
        let progress = context.state.lock().await.progress.clone();
        if context
            .state
            .lock()
            .await
            .last_response
            .starts_with("INPUT_REQUIRED:")
            || context.state.lock().await.repair_stopped
            || latest.outcome != QualityOutcome::Failed
            || latest.findings.is_empty()
            || progress.quality_round >= context.spec.limits.quality_repair_limit
            || progress.coding_turns_consumed >= context.spec.limits.coding_turns
            || context.coding_time().is_err()
        {
            return freeze(context, latest).await;
        }
        context.state.lock().await.progress.quality_round += 1;
        context.phase(WorkerPhase::Repairing).await?;
        context.save_source().await?;
        let findings = super::tools::summary(&latest)?;
        match runner.repair(context, &findings).await {
            Ok(Some(question)) => {
                context.state.lock().await.last_response = format!("INPUT_REQUIRED: {question}");
            }
            Ok(None) => {}
            Err(error) => {
                context.activity.fail(super::activity::io_failure(&error));
                context.state.lock().await.repair_stopped = true;
                context.phase(WorkerPhase::Checking).await?;
            }
        }
        let spec = context.spec.clone();
        let workspace = context.workspace.clone();
        tokio::task::spawn_blocking(move || {
            commits::commit_changes(&spec, &workspace, "fix(task): repair full quality findings")
        })
        .await
        .map_err(io::Error::other)??;
        context.save_source().await?;
    }
}
async fn freeze(context: &Context, result: QualityResult) -> io::Result<QualityResult> {
    context.phase(WorkerPhase::Finalizing).await?;
    context.activity.record(Stage::Finalization);
    super::environment::freeze(context)?;
    let report_path = context.output.join("evidence/report.json");
    if report_path.exists() {
        std::fs::remove_file(&report_path)?;
    }
    if let Some(evidence) = &result.evidence {
        let bytes = read_bounded(
            &context.workspace.join(".ayni/last/signals.json"),
            ayni_orchestrator_profile_ayni::managed_runtime::MAX_SIGNALS_BYTES,
        )?;
        let commit = crate::operations::runtime::git(&context.workspace, &["rev-parse", "HEAD"])?;
        if ayni_orchestrator_domain::digest::sha256(&bytes) != evidence.signals_digest
            || commit != evidence.commit
        {
            return Err(io::Error::other(
                "final check evidence does not match final commit",
            ));
        }
        recovery::atomic_write(&context.output.join("evidence/signals.json"), &bytes)?;
        let pending = context.output.join("quality-report.pending.json");
        if pending.exists() {
            let report: super::report::Report = super::report::Report::read(&pending)?;
            if report.matches(evidence) {
                recovery::write_json(&report_path, &report)?;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
