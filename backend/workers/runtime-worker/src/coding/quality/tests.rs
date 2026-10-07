use super::*;
use crate::operations::runtime::git;
use ayni_orchestrator_runtime::coding::{QualityEvidence, QualityRow};
use std::{
    collections::VecDeque,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
struct MockRunner {
    outcomes: Mutex<VecDeque<QualityOutcome>>,
    checks: AtomicUsize,
    repairs: AtomicUsize,
    check_edit: bool,
    repair_edit: bool,
    input_request: bool,
    repair_error: Option<&'static str>,
    repair_path: Option<&'static str>,
}
impl MockRunner {
    fn new(outcomes: impl IntoIterator<Item = QualityOutcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            checks: AtomicUsize::new(0),
            repairs: AtomicUsize::new(0),
            check_edit: false,
            repair_edit: false,
            input_request: false,
            repair_error: None,
            repair_path: None,
        }
    }
}
impl Runner for MockRunner {
    async fn check(&self, context: &Context, _: Duration) -> io::Result<QualityResult> {
        let index = self.checks.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .outcomes
            .lock()
            .map_err(|_| io::Error::other("mock poisoned"))?
            .pop_front()
            .ok_or_else(|| io::Error::other("unexpected check"))?;
        if self.check_edit && index == 0 {
            fs::write(
                context.workspace.join("feature.txt"),
                "check-generated formatting\n",
            )?;
        }
        if outcome == QualityOutcome::Unavailable {
            return Ok(unavailable("runner unavailable"));
        }
        let commit = git(&context.workspace, &["rev-parse", "HEAD"])?;
        let bytes = serde_json::to_vec(&serde_json::json!({"commit": commit, "check": index}))?;
        recovery::atomic_write(&context.workspace.join(".ayni/last/signals.json"), &bytes)?;
        Ok(QualityResult {
            evidence: Some(QualityEvidence {
                artifact_path: "evidence/signals.json".into(),
                signals_digest: ayni_orchestrator_domain::digest::sha256(&bytes),
                source_fingerprint: format!("sha256:{}", "a".repeat(64)),
                commit,
            }),
            completed_targets: Some(1),
            skipped_targets: Some(0),
            warning_offenders: Some(0),
            failing_offenders: Some(u32::from(outcome == QualityOutcome::Failed)),
            rows: vec![QualityRow {
                kind: "test".into(),
                passed: outcome == QualityOutcome::Passed,
            }],
            findings: if outcome == QualityOutcome::Failed {
                vec!["repair the failing test".into()]
            } else {
                vec![]
            },
            outcome,
        })
    }
    async fn repair(&self, context: &Context, findings: &str) -> io::Result<Option<String>> {
        assert!(findings.contains("repair the failing test"));
        self.repairs.fetch_add(1, Ordering::SeqCst);
        context.state.lock().await.progress.coding_turns_consumed += 1;
        if self.repair_edit {
            fs::write(context.workspace.join("feature.txt"), "repair changes\n")?;
        }
        if let Some(path) = self.repair_path {
            fs::write(context.workspace.join(path), "partial repair edits\n")?;
        }
        if let Some(error) = self.repair_error {
            return Err(io::Error::other(error));
        }
        Ok(self
            .input_request
            .then(|| "Which behavior should be selected?".into()))
    }
}

fn context(root: &std::path::Path, repair_limit: u8) -> io::Result<Context> {
    let workspace = root.join("workspace");
    let mut spec = super::super::tests::fixture(&workspace)?;
    spec.limits.coding_turns = 24;
    spec.limits.quality_repair_limit = repair_limit;
    Ok(Context {
        spec: Arc::new(spec),
        workspace,
        output: root.join("output"),
        state: Arc::new(tokio::sync::Mutex::new(
            super::super::sessions::SessionState::default(),
        )),
        serial: Arc::new(tokio::sync::Mutex::new(())),
        activity: super::super::activity::ActivityLog::default(),
    })
}

#[tokio::test]
async fn mandatory_full_check_runs_without_agent_tools_and_freezes_exact_commit() -> TestResult {
    let directory = tempfile::tempdir()?;
    let context = context(directory.path(), 3)?;
    let runner = MockRunner::new([QualityOutcome::Passed]);
    let result = coordinate_with(&context, &runner).await?;
    assert_eq!(runner.checks.load(Ordering::SeqCst), 1);
    assert_eq!(runner.repairs.load(Ordering::SeqCst), 0);
    assert_eq!(
        git(&context.workspace, &["rev-list", "--count", "HEAD"])?,
        "1"
    );
    let evidence = result.evidence.ok_or("evidence missing")?;
    assert_eq!(
        evidence.commit,
        git(&context.workspace, &["rev-parse", "HEAD"])?
    );
    assert_eq!(
        evidence.signals_digest,
        ayni_orchestrator_domain::digest::sha256(&fs::read(
            context.output.join("evidence/signals.json")
        )?)
    );
    Ok(())
}

#[tokio::test]
async fn zero_and_three_repair_limits_bound_sessions_without_empty_commits() -> TestResult {
    for limit in [0_u8, 3] {
        let directory = tempfile::tempdir()?;
        let context = context(directory.path(), limit)?;
        let runner = MockRunner::new(std::iter::repeat_n(
            QualityOutcome::Failed,
            usize::from(limit) + 1,
        ));
        assert_eq!(
            coordinate_with(&context, &runner).await?.outcome,
            QualityOutcome::Failed
        );
        assert_eq!(runner.checks.load(Ordering::SeqCst), usize::from(limit) + 1);
        assert_eq!(runner.repairs.load(Ordering::SeqCst), usize::from(limit));
        assert_eq!(context.state.lock().await.progress.quality_round, limit);
        assert_eq!(
            git(&context.workspace, &["rev-list", "--count", "HEAD"])?,
            "1"
        );
    }
    Ok(())
}

#[tokio::test]
async fn operational_retries_do_not_use_source_repair_sessions_or_zero_measurements() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let context = context(directory.path(), 3)?;
    let runner = MockRunner::new(std::iter::repeat_n(QualityOutcome::Unavailable, 3));
    let result = coordinate_with(&context, &runner).await?;
    assert_eq!(runner.checks.load(Ordering::SeqCst), 3);
    assert_eq!(runner.repairs.load(Ordering::SeqCst), 0);
    assert_eq!(result.outcome, QualityOutcome::Unavailable);
    assert_eq!(result.completed_targets, None);
    assert_eq!(result.evidence, None);
    Ok(())
}

#[tokio::test]
async fn check_generated_source_is_committed_and_checked_again_before_evidence_freezes()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let context = context(directory.path(), 3)?;
    let mut runner = MockRunner::new([QualityOutcome::Passed, QualityOutcome::Passed]);
    runner.check_edit = true;
    let result = coordinate_with(&context, &runner).await?;
    assert_eq!(runner.checks.load(Ordering::SeqCst), 2);
    assert_eq!(runner.repairs.load(Ordering::SeqCst), 0);
    assert_eq!(
        git(&context.workspace, &["rev-list", "--count", "HEAD"])?,
        "2"
    );
    assert_eq!(
        result.evidence.ok_or("evidence")?.commit,
        git(&context.workspace, &["rev-parse", "HEAD"])?
    );
    assert!(git(&context.workspace, &["diff", "--name-only", "HEAD"])?.is_empty());
    Ok(())
}

#[tokio::test]
async fn repair_input_requests_commit_real_edits_and_recheck_before_stopping_repairs() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let context = context(directory.path(), 3)?;
    let mut runner = MockRunner::new([QualityOutcome::Failed, QualityOutcome::Failed]);
    runner.repair_edit = true;
    runner.input_request = true;
    let result = coordinate_with(&context, &runner).await?;
    assert_eq!(runner.checks.load(Ordering::SeqCst), 2);
    assert_eq!(runner.repairs.load(Ordering::SeqCst), 1);
    assert_eq!(
        git(&context.workspace, &["rev-list", "--count", "HEAD"])?,
        "2"
    );
    assert!(
        context
            .state
            .lock()
            .await
            .last_response
            .starts_with("INPUT_REQUIRED:")
    );
    assert_eq!(
        result.evidence.ok_or("evidence")?.commit,
        git(&context.workspace, &["rev-parse", "HEAD"])?
    );
    assert!(git(&context.workspace, &["diff", "--name-only", "HEAD"])?.is_empty());
    Ok(())
}

#[tokio::test]
async fn corrupted_artifact_or_changed_commit_cannot_be_frozen_as_valid_evidence() -> TestResult {
    let directory = tempfile::tempdir()?;
    let context = context(directory.path(), 0)?;
    let runner = MockRunner::new([QualityOutcome::Passed, QualityOutcome::Passed]);
    let result = runner.check(&context, Duration::from_secs(10)).await?;
    fs::write(
        context.workspace.join(".ayni/last/signals.json"),
        "corrupt artifact",
    )?;
    assert!(freeze(&context, result).await.is_err());
    let result = runner.check(&context, Duration::from_secs(10)).await?;
    fs::write(
        context.workspace.join("feature.txt"),
        "changed after checking\n",
    )?;
    commits::commit_changes(
        &context.spec,
        &context.workspace,
        "fix: alter checked source",
    )?;
    assert!(freeze(&context, result).await.is_err());
    assert!(!context.output.join("evidence/signals.json").exists());
    Ok(())
}

#[tokio::test]
async fn interrupted_repair_commits_permitted_partial_work_and_checks_final_commit() -> TestResult {
    for final_outcome in [QualityOutcome::Failed, QualityOutcome::Passed] {
        let directory = tempfile::tempdir()?;
        let context = context(directory.path(), 3)?;
        let mut runner = MockRunner::new([QualityOutcome::Failed, final_outcome.clone()]);
        runner.repair_edit = true;
        runner.repair_error = Some("repair model unavailable");
        let result = coordinate_with(&context, &runner).await?;
        assert_eq!(result.outcome, final_outcome);
        assert_eq!(runner.checks.load(Ordering::SeqCst), 2);
        assert_eq!(runner.repairs.load(Ordering::SeqCst), 1);
        assert_eq!(context.state.lock().await.progress.quality_round, 1);
        assert!(context.state.lock().await.repair_stopped);
        assert_eq!(
            git(&context.workspace, &["rev-list", "--count", "HEAD"])?,
            "2"
        );
        assert_eq!(
            result.evidence.ok_or("final evidence")?.commit,
            git(&context.workspace, &["rev-parse", "HEAD"])?
        );
        assert!(git(&context.workspace, &["diff", "--name-only", "HEAD"])?.is_empty());
        assert_eq!(
            fs::read_to_string(context.workspace.join("feature.txt"))?,
            "repair changes\n"
        );
    }
    Ok(())
}

#[tokio::test]
async fn interrupted_repair_with_protected_partial_edits_remains_fail_closed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let context = context(directory.path(), 3)?;
    let mut runner = MockRunner::new([QualityOutcome::Failed]);
    runner.repair_path = Some("protected.txt");
    runner.repair_error = Some("repair model unavailable");
    assert!(coordinate_with(&context, &runner).await.is_err());
    assert_eq!(runner.checks.load(Ordering::SeqCst), 1);
    assert_eq!(runner.repairs.load(Ordering::SeqCst), 1);
    assert_eq!(
        git(&context.workspace, &["rev-parse", "HEAD"])?,
        context.spec.execution.revision
    );
    assert!(!context.output.join("evidence/signals.json").exists());
    Ok(())
}
