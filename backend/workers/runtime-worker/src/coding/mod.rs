//! Bounded coding in an already verified runtime. Outputs are advisory, not assessments.
use crate::{
    coding::activity::ActivityLog,
    coding::model_usage::{TrackedModel, UsageTracker},
    operations::runtime::{command, git},
    profiles::ayni::verify,
};
use ayni_orchestrator_domain::ModelUsageStage;
use ayni_orchestrator_profile_ayni::coding::CodingRequest;
use ayni_orchestrator_runtime::{
    activity::Stage, coding::CodingReport, runtime_execution::read_bounded,
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

pub const ADVISORY_INSTRUCTIONS: &str = "Read AGENTS.md and repository instructions. Make only the requested changes. Run plain `ayni check` before completing and repair actionable failures. Never weaken .ayni.toml or .ayni.lock merely to pass. Coding-time checks are advisory even when Ayni reports managed execution; only the later exact-head PR assessment is authoritative. Do not build an environment or access GitHub. The task runner checkpoints your edits and automatically commits and publishes the saved work as a draft, even if checks or the model fail. Run the final Ayni check before handing off; report remaining failures truthfully. If essential information is missing, respond INPUT_REQUIRED: followed by one concrete question.";

#[derive(Clone)]
struct RunCommand {
    request: std::sync::Arc<CodingRequest>,
    output: std::path::PathBuf,
    activity: ActivityLog,
    serial: std::sync::Arc<tokio::sync::Mutex<()>>,
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
        let _guard = self.serial.lock().await;
        let result = run_command(args).await;
        crate::coding::checkpoint::retain(
            &self.request,
            Path::new("/workspace"),
            &self.output,
            &self.activity,
        );
        result
    }
}

async fn run_command(args: CommandArgs) -> io::Result<String> {
    if args.command.is_empty() || args.command.len() > 16384 {
        return Err(io::Error::other("command exceeds bound"));
    }
    let mut process =
        tokio::process::Command::from(command("/usr/bin/timeout", Path::new("/workspace")));
    let mut child = process
        .args(["--kill-after=5", "180", "/bin/sh", "-c", &args.command])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr missing"))?;
    let (status, stdout, stderr) =
        tokio::join!(child.wait(), bounded_output(stdout), bounded_output(stderr));
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
    // This runs before the model gets any tools. A quality failure is still advisory.
    activity.record(Stage::RuntimeVerification);
    verify(&request.execution, workspace)?;
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
    let agent = TrackedModel::new(client.completion_model(&request.model), usage.clone())
        .into_agent_builder()
        .preamble(ADVISORY_INSTRUCTIONS)
        .tool(RunCommand {
            request: std::sync::Arc::new(request.clone()),
            output: output.to_owned(),
            activity: activity.clone(),
            serial: std::sync::Arc::new(tokio::sync::Mutex::new(())),
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
        .max_turns(24)
        .await;
    // Finalize recovery output even when the provider or turn budget failed.
    crate::coding::checkpoint::retain(request, workspace, output, activity);
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
            advisory: "Coding-time checks are advisory; no candidate produced.".into(),
        }
    } else {
        activity.record(Stage::CandidateFreeze);
        freeze(request, workspace, output)?
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

const fn advisory_outcome(exit: Option<i32>) -> &'static str {
    match exit {
        Some(0) => {
            "Coding-time Ayni check passed (advisory only). Managed PR assessment is still required."
        }
        Some(_) => {
            "Coding-time Ayni check failed (advisory only). Managed PR assessment is still required."
        }
        None => {
            "Coding-time Ayni check unavailable or incompatible with the frozen runtime (advisory only). Managed PR assessment is still required."
        }
    }
}

fn freeze(request: &CodingRequest, workspace: &Path, output: &Path) -> io::Result<CodingReport> {
    // Always do our own final advisory invocation: model prose is not a passing check.
    let advisory = advisory_outcome(
        verify(&request.execution, workspace)
            .ok()
            .map(|report| report.check_exit_code),
    );
    freeze_with_advisory(request, workspace, output, advisory)
}

pub(crate) fn freeze_with_advisory(
    request: &CodingRequest,
    workspace: &Path,
    output: &Path,
    advisory: &str,
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
        advisory: advisory.into(),
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
    fn prompt_never_claims_assessment_authority() {
        assert!(ADVISORY_INSTRUCTIONS.contains("advisory"));
        assert!(ADVISORY_INSTRUCTIONS.contains("Never weaken"));
        assert!(test_path("src/button.test.ts"));
    }

    #[test]
    fn check_failure_or_absence_never_becomes_success() {
        assert!(advisory_outcome(Some(0)).contains("passed"));
        assert!(advisory_outcome(Some(1)).contains("failed"));
        assert!(advisory_outcome(None).contains("unavailable"));
        for outcome in [Some(0), Some(1), None] {
            assert!(advisory_outcome(outcome).contains("advisory only"));
        }
    }

    #[test]
    fn freezes_bundle_without_importing_agent_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let output = tempfile::tempdir()?;
        let workspace = root.path();
        let revision = initialize_workspace(workspace)?;
        stage_candidate_with_untrusted_evidence(workspace, &revision)?;
        let request = coding_request(revision);
        assert_checkpoint_recovery(&request, workspace, output.path())?;

        // Finalization after a checkpoint still emits a complete candidate.
        let report = freeze(&request, workspace, output.path())?;
        assert_eq!(report.changed_paths, ["file.txt"]);
        assert!(report.advisory.contains("unavailable"));
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
        assert!(checkpoint.advisory.contains("partial work"));
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
