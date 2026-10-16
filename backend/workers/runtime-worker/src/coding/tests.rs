use super::*;
use ayni_orchestrator_domain::ChangeConstraints;
use ayni_orchestrator_profile_ayni::{
    managed_runtime::RuntimeExpectation, runtime_execution::ExecutionRequest,
};
use ayni_orchestrator_runtime::coding::{PublicationDestination, WorkerLimits};
use std::time::{SystemTime, UNIX_EPOCH};

type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(super) fn fixture(workspace: &Path) -> io::Result<WorkerSpec> {
    fs::create_dir_all(workspace)?;
    git(workspace, &["init", "--quiet"])?;
    fs::write(workspace.join("feature.txt"), "baseline\n")?;
    fs::write(workspace.join("protected.txt"), "policy\n")?;
    git(workspace, &["add", "--all"])?;
    git(
        workspace,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Worker Test",
            "-c",
            "user.email=worker@invalid.example",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "chore: initialize test",
        ],
    )?;
    Ok(WorkerSpec {
        profile: None,
        task_id: "task-test".into(),
        attempt_id: "attempt-test".into(),
        execution: ExecutionRequest {
            repository_url: "https://github.com/team/repository.git".into(),
            revision: git(workspace, &["rev-parse", "HEAD"])?,
            expected: RuntimeExpectation {
                lock_fingerprint: format!("sha256:{}", "a".repeat(64)),
                platform: "linux/amd64".into(),
                ayni_version: "0.14.5".into(),
            },
        },
        runtime_digest: format!("sha256:{}", "b".repeat(64)),
        request: "Add a feature".into(),
        acceptance_criteria: vec!["It works".into()],
        constraints: ChangeConstraints {
            protected_paths: vec!["protected.txt".into()],
            allow_test_edits: true,
            environment_change: None,
        },
        provider_credentials: None,
        model: "test-model".into(),
        limits: WorkerLimits {
            coding_turns: 2,
            quality_repair_limit: 3,
            deadline_at: i64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(io::Error::other)?
                    .as_secs(),
            )
            .map_err(io::Error::other)?
                + 3600,
        },
        publication: PublicationDestination {
            repository: "team/repository".into(),
            base_branch: "main".into(),
            branch: "task/attempt-test".into(),
        },
        publication_only: false,
        reporting: None,
    })
}

#[test]
fn logical_commits_preserve_milestones_and_skip_empty_commits() -> TestResult {
    let directory = tempfile::tempdir()?;
    let spec = fixture(directory.path())?;
    fs::write(directory.path().join("feature.txt"), "feature one\n")?;
    let first = commits::commit_changes(&spec, directory.path(), "feat: add first milestone")?
        .ok_or("first commit missing")?;
    fs::write(directory.path().join("feature-two.txt"), "feature two\n")?;
    let second = commits::commit_changes(&spec, directory.path(), "feat: add second milestone")?
        .ok_or("second commit missing")?;
    assert_ne!(first, second);
    assert_eq!(git(directory.path(), &["rev-parse", "HEAD^"])?, first);
    assert_eq!(
        git(directory.path(), &["rev-list", "--count", "HEAD"])?,
        "3"
    );
    assert!(commits::commit_changes(&spec, directory.path(), "chore: finish work")?.is_none());
    assert_eq!(git(directory.path(), &["rev-parse", "HEAD"])?, second);
    assert_eq!(
        commits::changed_paths(&spec, directory.path())?,
        vec!["feature-two.txt", "feature.txt"]
    );
    Ok(())
}

#[test]
fn protected_edits_are_rejected_before_commit() -> TestResult {
    let directory = tempfile::tempdir()?;
    let spec = fixture(directory.path())?;
    fs::write(directory.path().join("protected.txt"), "weakened policy\n")?;
    assert!(commits::commit_changes(&spec, directory.path(), "fix: change policy").is_err());
    assert_eq!(
        git(directory.path(), &["rev-parse", "HEAD"])?,
        spec.execution.revision
    );
    Ok(())
}

#[test]
fn atomic_recovery_retains_history_source_and_consumed_budget_without_commits() -> TestResult {
    let directory = tempfile::tempdir()?;
    let workspace = directory.path().join("workspace");
    let output = directory.path().join("output");
    let spec = fixture(&workspace)?;
    fs::write(workspace.join("feature.txt"), "committed feature\n")?;
    let head = commits::commit_changes(&spec, &workspace, "feat: add milestone")?
        .ok_or("commit missing")?;
    fs::write(workspace.join("feature.txt"), "repair in progress\n")?;
    fs::write(workspace.join("untracked.txt"), "unfinished source\n")?;
    let mut state = SessionState::default();
    state.progress.phase = WorkerPhase::Repairing;
    state.progress.coding_turns_consumed = 1;
    state.progress.quality_round = 1;
    state.prior_conversation = "Original task and repair findings".into();
    recovery::save(&spec, &workspace, &output, &state)?;
    assert_eq!(git(&workspace, &["rev-parse", "HEAD"])?, head);
    let mut newer = state.clone();
    newer.progress.coding_turns_consumed = 2;
    newer.repair_stopped = true;
    recovery::write_json(&output.join("session.json"), &newer)?;
    fs::write(workspace.join("feature.txt"), "lost edits\n")?;
    fs::remove_file(workspace.join("untracked.txt"))?;
    let mut replacement = spec.clone();
    replacement.reporting = Some(ayni_orchestrator_runtime::coding_events::CodingAdmission {
        configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
            source: "ayni://orchestrator/supervisors/replacement".into(),
            endpoint: "http://controller/events/verification".into(),
        },
        binding: ayni_orchestrator_events::v1::CodingExecutionBinding {
            generation: "replacement-generation".into(),
            ..Default::default()
        },
    });
    let restored =
        recovery::restore(&replacement, &workspace, &output)?.ok_or("recovery missing")?;
    assert_eq!(git(&workspace, &["rev-parse", "HEAD"])?, head);
    assert_eq!(
        git(&workspace, &["rev-parse", "HEAD^"])?,
        spec.execution.revision
    );
    assert_eq!(
        fs::read_to_string(workspace.join("feature.txt"))?,
        "repair in progress\n"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("untracked.txt"))?,
        "unfinished source\n"
    );
    assert_eq!(restored.progress.coding_turns_consumed, 2);
    assert!(restored.repair_stopped);
    assert_eq!(restored.progress.quality_round, 1);
    assert_eq!(restored.progress.phase, WorkerPhase::Repairing);
    assert_eq!(restored.prior_conversation, state.prior_conversation);
    let mut wrong = spec;
    wrong.attempt_id = "different-attempt".into();
    assert!(recovery::restore(&wrong, &workspace, &output).is_err());
    assert_eq!(git(&workspace, &["rev-parse", "HEAD"])?, head);
    Ok(())
}
