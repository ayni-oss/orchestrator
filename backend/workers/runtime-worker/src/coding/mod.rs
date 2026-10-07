//! Bounded coding in an already verified runtime. Outputs are advisory, not assessments.
use crate::{
    coding::activity::ActivityLog,
    coding::model_usage::{TrackedModel, UsageTracker},
    operations::runtime::{command, git},
    profiles::ayni::{preflight, verify},
};
use ayni_orchestrator_domain::ModelUsageStage;
use ayni_orchestrator_profile_ayni::coding::CodingRequest;
use ayni_orchestrator_runtime::{
    activity::Stage,
    coding::{CodingReport, ValidationOutcome, ValidationResult},
    runtime_execution::read_bounded,
};
use rig_agent::{
    client::AgentModelExt,
    completion::Prompt,
    tool::{Tool, ToolContext},
};
use rig_core::{client::completion::CompletionClient, providers::openai};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path, process::Stdio};
use tokio::io::AsyncReadExt;

pub const ADVISORY_INSTRUCTIONS: &str = "Read AGENTS.md and repository instructions. Make only the requested changes. Use the `ayni_validate` tool to run the full repository check while coding and repair actionable failures. Never weaken .ayni.toml or .ayni.lock merely to pass. When ready, call `finalize_candidate`; it runs the final check and freezes the exact candidate. Do not build an environment or access GitHub. If final validation fails, finalize truthfully so the draft PR shows the result. If essential information is missing, respond INPUT_REQUIRED: followed by one concrete question.";

#[derive(Default)]
struct CodingState {
    validation: Option<ValidationResult>,
    final_report: Option<CodingReport>,
}

#[derive(Clone)]
struct RunCommand {
    request: std::sync::Arc<CodingRequest>,
    output: std::path::PathBuf,
    activity: ActivityLog,
    serial: std::sync::Arc<tokio::sync::Mutex<()>>,
    state: std::sync::Arc<tokio::sync::Mutex<CodingState>>,
}
#[derive(Deserialize)]
struct CommandArgs {
    command: String,
}
impl Tool for RunCommand {
    const NAME: &'static str = "run_command";
    type Args = CommandArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Run a shell command in /workspace; use this to inspect and edit files. Output is bounded and each command has a 180 second deadline.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: CommandArgs) -> io::Result<String> {
        // Serialize commands and checkpointing so parallel tool calls cannot race Git.
        let guard = self.serial.clone().lock_owned().await;
        if self.state.lock().await.final_report.is_some() {
            return Err(io::Error::other("candidate already finalized"));
        }
        let result = run_command(args).await;
        crate::coding::checkpoint::retain_async(
            self.request.as_ref().clone(),
            Path::new("/workspace").to_owned(),
            self.output.clone(),
            self.activity.clone(),
            Some(guard),
        )
        .await?;
        result
    }
}

#[derive(Clone)]
struct AyniValidate {
    request: std::sync::Arc<CodingRequest>,
    state: std::sync::Arc<tokio::sync::Mutex<CodingState>>,
    serial: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl Tool for AyniValidate {
    const NAME: &'static str = "ayni_validate";
    type Args = ();
    type Output = String;
    type Error = io::Error;

    fn description(&self) -> String {
        "Run the pinned full Ayni check in /workspace and return a bounded structured summary."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }

    async fn call(&self, _: &mut ToolContext, _: ()) -> io::Result<String> {
        let _guard = self.serial.lock().await;
        if self.state.lock().await.final_report.is_some() {
            return Err(io::Error::other("candidate already finalized"));
        }
        let report = verify(&self.request.execution, Path::new("/workspace")).await?;
        let validation = report.validation;
        let output = validation_summary(&validation);
        self.state.lock().await.validation = Some(validation);
        Ok(output)
    }
}

#[derive(Clone)]
struct FinalizeCandidate {
    request: std::sync::Arc<CodingRequest>,
    output: std::path::PathBuf,
    activity: ActivityLog,
    state: std::sync::Arc<tokio::sync::Mutex<CodingState>>,
    serial: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl Tool for FinalizeCandidate {
    const NAME: &'static str = "finalize_candidate";
    type Args = ();
    type Output = String;
    type Error = io::Error;

    fn description(&self) -> String {
        "Run the final full Ayni check and atomically freeze the exact candidate for publication."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }

    async fn call(&self, _: &mut ToolContext, _: ()) -> io::Result<String> {
        let _guard = self.serial.lock().await;
        if self.state.lock().await.final_report.is_some() {
            return Err(io::Error::other("candidate already finalized"));
        }
        self.activity.record(Stage::CandidateFreeze);
        let validation = match verify(&self.request.execution, Path::new("/workspace")).await {
            Ok(report) => report.validation,
            Err(_) => unavailable_validation(),
        };
        let request = self.request.clone();
        let output = self.output.clone();
        let validation_for_freeze = validation.clone();
        let report = tokio::task::spawn_blocking(move || {
            freeze_with_validation(
                &request,
                Path::new("/workspace"),
                &output,
                validation_for_freeze,
            )
        })
        .await
        .map_err(io::Error::other)??;
        let summary = validation_summary(&validation);
        let mut state = self.state.lock().await;
        state.validation = Some(validation);
        state.final_report = Some(report);
        Ok(summary)
    }
}

async fn run_command(args: CommandArgs) -> io::Result<String> {
    if args.command.is_empty() || args.command.len() > 16384 {
        return Err(io::Error::other("command exceeds bound"));
    }
    let mut process =
        tokio::process::Command::from(command("/usr/bin/timeout", Path::new("/workspace")));
    process
        .args(["--kill-after=5", "180", "/bin/sh", "-c", &args.command])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::operations::process::spawn(&mut process)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr missing"))?;
    let (status, stdout, stderr) =
        tokio::time::timeout(std::time::Duration::from_secs(185), async {
            tokio::join!(child.wait(), bounded_output(stdout), bounded_output(stderr))
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "worker command output deadline exceeded",
            )
        })?;
    Ok(format!(
        "exit={:?}\nstdout:\n{}\nstderr:\n{}",
        status?.code(),
        stdout?,
        stderr?
    ))
}

async fn bounded_output(mut stream: impl tokio::io::AsyncRead + Unpin) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let keep = read.min(32000_usize.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Verifies the frozen runtime, runs the bounded agent, and freezes a Git bundle.
///
/// # Errors
/// Fails closed for runtime mismatch, provider failure, unsafe candidate, or output limits.
pub async fn run(
    request: &CodingRequest,
    output: &Path,
    activity: &ActivityLog,
) -> Result<(), Box<dyn std::error::Error>> {
    request.constraints.validate()?;
    if request.request.is_empty()
        || request.request.len() > 16384
        || request.model.len() > 128
        || request.acceptance_criteria.len() > 20
        || request
            .acceptance_criteria
            .iter()
            .map(String::len)
            .sum::<usize>()
            > 16384
    {
        return Err("invalid coding request".into());
    }
    let workspace = Path::new("/workspace");
    // Runtime identity is checked before the model gets tools. Quality checks are
    // deliberately performed by the explicit Ayni tools during coding.
    activity.record(Stage::RuntimeVerification);
    preflight(&request.execution, workspace)?;
    activity.record(Stage::OutputPreparation);
    fs::create_dir_all(output)?;
    let usage = UsageTracker::new(
        output.join("usage"),
        ModelUsageStage::Coding,
        "openai",
        &request.model,
    )?;
    activity.record(Stage::ModelCredentials);
    let key = read_bounded(Path::new("/model/api-key"), 16384)?;
    let client = openai::Client::new(std::str::from_utf8(&key)?.trim())?;
    let state = std::sync::Arc::new(tokio::sync::Mutex::new(CodingState::default()));
    let serial = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let agent = TrackedModel::new(client.completion_model(&request.model), usage.clone())
        .into_agent_builder()
        .preamble(ADVISORY_INSTRUCTIONS)
        .tool(RunCommand {
            request: std::sync::Arc::new(request.clone()),
            output: output.to_owned(),
            activity: activity.clone(),
            serial: serial.clone(),
            state: state.clone(),
        })
        .tool(AyniValidate {
            request: std::sync::Arc::new(request.clone()),
            state: state.clone(),
            serial: serial.clone(),
        })
        .tool(FinalizeCandidate {
            request: std::sync::Arc::new(request.clone()),
            output: output.to_owned(),
            activity: activity.clone(),
            state: state.clone(),
            serial,
        })
        .add_hook(activity.clone())
        .build();
    activity.record(Stage::ModelExecution);
    let response = agent
        .prompt(format!(
            "Request: {}\nAcceptance: {}\nFrozen constraints: {}",
            request.request,
            serde_json::to_string(&request.acceptance_criteria)?,
            serde_json::to_string(&request.constraints)?
        ))
        .max_turns(usize::from(request.max_turns))
        .await;
    if let Err(error) = &response {
        activity.record(Stage::ModelExecution);
        activity.fail(crate::coding::activity::prompt_failure(error));
        usage.fail_pending(None)?;
    }
    let response = response?;
    let report = if let Some(question) = response.trim().strip_prefix("INPUT_REQUIRED:") {
        if question.trim().is_empty() {
            return Err("input request has no question".into());
        }
        CodingReport {
            commit: None,
            bundle_digest: None,
            changed_paths: vec![],
            requested_input: Some(question.trim().chars().take(256).collect()),
            validation: missing_validation(),
        }
    } else {
        let finalized = state.lock().await.final_report.clone();
        if let Some(report) = finalized {
            report
        } else {
            activity.record(Stage::CandidateFreeze);
            let request = request.clone();
            let workspace = workspace.to_owned();
            let output = output.to_owned();
            tokio::task::spawn_blocking(move || {
                freeze_with_validation(&request, &workspace, &output, missing_validation())
            })
            .await
            .map_err(io::Error::other)??
        }
    };
    activity.record(Stage::ReportWrite);
    let bytes = serde_json::to_vec(&report)?;
    if bytes.len() > 65536 {
        return Err("coding result oversized".into());
    }
    fs::write(output.join("report.pending"), bytes)?;
    fs::rename(output.join("report.pending"), output.join("report.json"))?;
    Ok(())
}

pub(crate) fn missing_validation() -> ValidationResult {
    ValidationResult {
        outcome: ValidationOutcome::Missing,
        signals_digest: None,
        completed_targets: 0,
        skipped_targets: 0,
        warning_offenders: 0,
        failing_offenders: 0,
        rows: vec![],
    }
}

fn unavailable_validation() -> ValidationResult {
    ValidationResult {
        outcome: ValidationOutcome::Unavailable,
        ..missing_validation()
    }
}

fn validation_summary(validation: &ValidationResult) -> String {
    format!(
        "outcome={:?}; targets={} completed, {} skipped; warnings={}; failures={}",
        validation.outcome,
        validation.completed_targets,
        validation.skipped_targets,
        validation.warning_offenders,
        validation.failing_offenders,
    )
}

pub(crate) fn freeze_with_validation(
    request: &CodingRequest,
    workspace: &Path,
    output: &Path,
    validation: ValidationResult,
) -> io::Result<CodingReport> {
    // Never include coding-generated Ayni evidence or launcher output in candidates.
    git(workspace, &["read-tree", &request.execution.revision])?;
    git(
        workspace,
        &["-c", "core.hooksPath=/dev/null", "add", "--all", "--", "."],
    )?;
    // Exclusion pathspecs naming ignored directories make `git add` fail.
    // Restore these paths in the index after staging, without forcing ignored files.
    git(
        workspace,
        &[
            "reset",
            "--quiet",
            &request.execution.revision,
            "--",
            ".ayni",
            ".orchestrator",
        ],
    )?;
    let changed = git(
        workspace,
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            &request.execution.revision,
        ],
    )?;
    let paths: Vec<String> = changed
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    if paths.is_empty() || paths.len() > 256 || paths.iter().map(String::len).sum::<usize>() > 32768
    {
        return Err(io::Error::other("candidate paths outside bounds"));
    }
    for path in &paths {
        if !ayni_orchestrator_domain::repository_path_is_valid(path)
            || request.constraints.protecting_path(path).is_some()
        {
            return Err(io::Error::other("candidate changes a protected path"));
        }
        if !request.constraints.allow_test_edits
            && (test_path(path)
                || (Path::new(path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
                    && rust_test_file_changed(workspace, path, &request.execution.revision)))
        {
            return Err(io::Error::other("candidate test edits are not authorized"));
        }
    }
    git(
        workspace,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Ayni Agent",
            "-c",
            "user.email=agent@invalid.example",
            "commit",
            "--allow-empty",
            "--quiet",
            "-s",
            "-m",
            "chore(agent): apply requested change",
        ],
    )?;
    let commit = git(workspace, &["rev-parse", "HEAD"])?;
    let bundle = output.join("candidate.bundle");
    git(
        workspace,
        &[
            "bundle",
            "create",
            bundle
                .to_str()
                .ok_or_else(|| io::Error::other("invalid output path"))?,
            "HEAD",
            &format!("^{}", request.execution.revision),
        ],
    )?;
    let bytes = read_bounded(&bundle, 64 * 1024 * 1024)?;
    Ok(CodingReport {
        commit: Some(commit),
        bundle_digest: Some(format!("sha256:{:x}", Sha256::digest(bytes))),
        changed_paths: paths,
        requested_input: None,
        validation,
    })
}

fn test_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "test" | "tests" | "__tests__"))
        || path.contains(".test.")
        || path.contains(".spec.")
        || path.ends_with("_test.go")
        || path.starts_with("test_")
}

fn rust_test_file_changed(workspace: &Path, path: &str, base: &str) -> bool {
    // Conservative until a language-specific syntax-aware policy is selected:
    // files containing inline tests require explicit test-edit permission.
    let Ok(old_paths) = git(workspace, &["ls-tree", "--name-only", base, "--", path]) else {
        return true;
    };
    let Ok(new_paths) = git(workspace, &["ls-files", "--", path]) else {
        return true;
    };
    let old = if old_paths.is_empty() {
        Ok(String::new())
    } else {
        git(workspace, &["show", &format!("{base}:{path}")])
    };
    let new = if new_paths.is_empty() {
        Ok(String::new())
    } else {
        git(workspace, &["show", &format!(":{path}")])
    };
    let (Ok(old), Ok(new)) = (old, new) else {
        return true;
    };
    old != new
        && [old, new]
            .iter()
            .any(|text| text.contains("#[test]") || text.contains("cfg(test)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn tool_output_is_bounded_without_blocking_drain() -> io::Result<()> {
        let bytes = vec![b'a'; 100_000];
        assert_eq!(bounded_output(bytes.as_slice()).await?.len(), 32000);
        Ok(())
    }
    #[test]
    fn prompt_requires_explicit_validation_and_finalization() {
        assert!(ADVISORY_INSTRUCTIONS.contains("ayni_validate"));
        assert!(ADVISORY_INSTRUCTIONS.contains("finalize_candidate"));
        assert!(ADVISORY_INSTRUCTIONS.contains("Never weaken"));
        assert!(test_path("src/button.test.ts"));
    }

    #[test]
    fn missing_validation_never_becomes_success() {
        assert_eq!(missing_validation().outcome, ValidationOutcome::Missing);
        assert_eq!(
            unavailable_validation().outcome,
            ValidationOutcome::Unavailable
        );
    }

    #[tokio::test]
    async fn freezes_bundle_without_importing_agent_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let output = tempfile::tempdir()?;
        let workspace = root.path();
        let revision = initialize_workspace(workspace)?;
        stage_candidate_with_untrusted_evidence(workspace, &revision)?;
        let request = coding_request(revision);
        assert_checkpoint_recovery(&request, workspace, output.path())?;

        // Finalization after a checkpoint still emits a complete candidate.
        let report =
            freeze_with_validation(&request, workspace, output.path(), unavailable_validation())?;
        assert_eq!(report.changed_paths, ["file.txt"]);
        assert_eq!(report.validation.outcome, ValidationOutcome::Unavailable);
        assert!(report.bundle_digest.is_some());
        assert!(
            !git(workspace, &["ls-tree", "-r", "--name-only", "HEAD"])?.contains("signals.json")
        );
        git(
            workspace,
            &[
                "bundle",
                "verify",
                output
                    .path()
                    .join("candidate.bundle")
                    .to_str()
                    .ok_or("path")?,
            ],
        )?;
        Ok(())
    }

    fn initialize_workspace(workspace: &Path) -> io::Result<String> {
        git(workspace, &["init", "--quiet"])?;
        fs::write(workspace.join("file.txt"), "before")?;
        fs::write(workspace.join(".gitignore"), "/.ayni/\n/target/\n")?;
        git(workspace, &["add", "."])?;
        git(
            workspace,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "test: base",
            ],
        )?;
        git(workspace, &["rev-parse", "HEAD"])
    }

    fn stage_candidate_with_untrusted_evidence(workspace: &Path, revision: &str) -> io::Result<()> {
        fs::write(workspace.join("new.rs"), "pub fn value() -> u8 { 1 }\n")?;
        git(workspace, &["add", "new.rs"])?;
        assert!(!rust_test_file_changed(workspace, "new.rs", revision));
        fs::write(workspace.join("new.rs"), "#[test] fn value() {}\n")?;
        git(workspace, &["add", "new.rs"])?;
        assert!(rust_test_file_changed(workspace, "new.rs", revision));
        git(workspace, &["rm", "--force", "new.rs"])?;
        fs::write(workspace.join("file.txt"), "after")?;
        fs::create_dir(workspace.join(".ayni"))?;
        fs::write(workspace.join(".ayni/signals.json"), "untrusted")?;
        // Even force-staged ignored evidence must not enter the candidate.
        git(workspace, &["add", "--force", ".ayni/signals.json"])?;
        git(workspace, &["add", "--all"])?;
        Ok(())
    }

    fn coding_request(revision: String) -> CodingRequest {
        CodingRequest {
            execution: ayni_orchestrator_profile_ayni::runtime_execution::ExecutionRequest {
                repository_url: "https://github.com/team/repo.git".into(),
                revision,
                expected: ayni_orchestrator_profile_ayni::managed_runtime::RuntimeExpectation {
                    lock_fingerprint: String::new(),
                    platform: "linux/amd64".into(),
                    ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION.into(),
                },
            },
            request: "edit".into(),
            acceptance_criteria: vec!["changed".into()],
            constraints: ayni_orchestrator_domain::ChangeConstraints {
                protected_paths: vec![],
                allow_test_edits: true,
            },
            model: "test".into(),
            max_turns: 24,
        }
    }

    fn assert_checkpoint_recovery(
        request: &CodingRequest,
        workspace: &Path,
        output: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        crate::coding::checkpoint::save(request, workspace, output)?;
        let first = fs::read_to_string(output.join("checkpoint"))?;
        fs::write(workspace.join("file.txt"), "latest edit")?;
        crate::coding::checkpoint::save(request, workspace, output)?;
        let latest = fs::read_to_string(output.join("checkpoint"))?;
        assert_ne!(first, latest);
        let saved = output.join("checkpoints").join(&latest);
        let checkpoint: CodingReport =
            serde_json::from_slice(&fs::read(saved.join("report.json"))?)?;
        assert_eq!(checkpoint.commit.as_deref(), Some(latest.as_str()));
        assert_eq!(fs::read_dir(output.join("checkpoints"))?.count(), 1);
        assert_eq!(checkpoint.validation.outcome, ValidationOutcome::Missing);
        git(
            workspace,
            &[
                "bundle",
                "verify",
                saved.join("candidate.bundle").to_str().ok_or("path")?,
            ],
        )?;
        let mut protected = request.clone();
        protected.constraints.protected_paths = vec!["file.txt".into()];
        assert!(crate::coding::checkpoint::save(&protected, workspace, output).is_err());
        assert_eq!(fs::read_to_string(output.join("checkpoint"))?, latest);
        Ok(())
    }
}

pub mod activity;
mod checkpoint;
pub mod model_usage;
