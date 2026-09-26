//! Bounded coding agent. It receives a checkout, a model key, and no
//! Kubernetes or GitHub App credentials.

use rig_agent::{
    client::AgentClientExt,
    completion::Prompt,
    tool::{Tool, ToolContext},
};
use rig_core::providers::openai;
use serde::Deserialize;
use serde_json::json;
use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
    time::Instant,
};
use tracing::{Instrument, error, info, info_span};

const MAX_TOOL_OUTPUT: usize = 32_000;
const CANDIDATE_COMMIT_TITLE: &str = "chore(agent): apply requested change";

#[derive(Clone)]
struct Workspace(PathBuf);

impl Workspace {
    fn path(&self, relative: &str) -> Result<PathBuf, io::Error> {
        let path = Path::new(relative);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid workspace path",
            ));
        }
        let parent = self
            .0
            .join(path)
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no parent"))?
            .canonicalize()?;
        if !parent.starts_with(&self.0) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path escapes workspace",
            ));
        }
        let resolved = parent.join(
            path.file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no name"))?,
        );
        if fs::symlink_metadata(&resolved).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "symlink file is not allowed",
            ));
        }
        Ok(resolved)
    }
}

#[derive(Deserialize)]
struct FileArgs {
    path: String,
}

#[derive(Clone)]
struct ReadFile(Workspace);

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";
    type Args = FileArgs;
    type Output = String;
    type Error = io::Error;

    fn description(&self) -> String {
        "Read a UTF-8 file in the task checkout".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    async fn call(
        &self,
        _: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let contents = fs::read_to_string(self.0.path(&args.path)?)?;
        Ok(truncate(contents))
    }
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[derive(Clone)]
struct WriteFile(Workspace);

impl Tool for WriteFile {
    const NAME: &'static str = "write_file";
    type Args = WriteArgs;
    type Output = String;
    type Error = io::Error;

    fn description(&self) -> String {
        "Replace a UTF-8 file in the task checkout".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    async fn call(
        &self,
        _: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        if args.content.len() > 1_000_000 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file exceeds 1 MiB",
            ));
        }
        fs::write(self.0.path(&args.path)?, args.content)?;
        Ok("file written".into())
    }
}

#[derive(Deserialize)]
struct CommandArgs {
    command: String,
}

#[derive(Clone)]
struct RunCommand(Workspace);

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";
    type Args = CommandArgs;
    type Output = String;
    type Error = io::Error;

    fn description(&self) -> String {
        "Run a command inside the isolated task checkout".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})
    }
    async fn call(
        &self,
        _: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        if args.command.len() > 2048 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command is too long",
            ));
        }
        let output = tokio::process::Command::new("sh")
            .arg("-lc")
            .arg(&args.command)
            .current_dir(&self.0.0)
            .env_clear()
            .env(
                "PATH",
                env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
            )
            .env("HOME", "/workspace")
            .env(
                "CARGO_HOME",
                env::var("CARGO_HOME").unwrap_or_else(|_| "/workspace/.cargo".into()),
            )
            .env(
                "RUSTUP_HOME",
                env::var("RUSTUP_HOME").unwrap_or_else(|_| "/usr/local/rustup".into()),
            )
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await?;
        Ok(format!(
            "exit={:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            truncate(String::from_utf8_lossy(&output.stdout).into_owned()),
            truncate(String::from_utf8_lossy(&output.stderr).into_owned())
        ))
    }
}

fn truncate(mut value: String) -> String {
    if value.len() > MAX_TOOL_OUTPUT {
        let end = value.floor_char_boundary(MAX_TOOL_OUTPUT);
        value.truncate(end);
        value.push_str("\n[truncated]");
    }
    value
}

fn git(workspace: &Path, args: &[&str]) -> Result<String, io::Error> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {:?} failed: {}",
            args,
            truncate(String::from_utf8_lossy(&output.stderr).into_owned())
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(env::var("DEVELOPMENT_WORKSPACE")?).canonicalize()?;
    let checkpoint = PathBuf::from(env::var("DEVELOPMENT_CHECKPOINT")?);
    let start_revision = env::var("DEVELOPMENT_START_REVISION")?;
    let request = env::var("DEVELOPMENT_REQUEST")?;
    let criteria = env::var("DEVELOPMENT_ACCEPTANCE_CRITERIA")?;
    let change_constraints = env::var("DEVELOPMENT_CHANGE_CONSTRAINTS")?;
    let model = env::var("AGENT_MODEL")?;
    let key = env::var("OPENAI_API_KEY")?;
    if git(&workspace, &["rev-parse", "HEAD"])? != start_revision {
        return Err("checkout does not match the task starting revision".into());
    }
    let client = openai::Client::new(&key)?;
    let agent = client.agent(&model)
        .preamble("You are a coding agent in an isolated checkout. Read AGENTS.md and repository guidance. Make only changes needed for the request, run focused checks, and report what you changed. Do not publish a branch or PR. The controller will independently validate your final candidate. If essential information is missing and no safe bounded change can be made, make no edits and respond exactly with INPUT_REQUIRED: followed by one concrete question.")
        .tool(ReadFile(Workspace(workspace.clone())))
        .tool(WriteFile(Workspace(workspace.clone())))
        .tool(RunCommand(Workspace(workspace.clone())))
        .build();
    let provider_span = info_span!(
        "provider.call",
        provider = "openai",
        model = %model,
        stage = "coding"
    );
    let response = async {
        let started = Instant::now();
        let result = agent
            .prompt(format!(
                "Request: {request}\nAcceptance criteria: {criteria}\nFrozen change constraints: {change_constraints}. Do not modify protected paths or tests unless these constraints explicitly allow it."
            ))
            .max_turns(24)
            .await;
        info!(
            outcome = if result.is_ok() { "succeeded" } else { "failed" },
            latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "provider call completed"
        );
        result
    }
    .instrument(provider_span)
    .await?;
    if let Some(question) = response.trim().strip_prefix("INPUT_REQUIRED:") {
        let question = question.trim();
        if question.is_empty() {
            return Err("agent requested input without a question".into());
        }
        fs::create_dir_all(&checkpoint)?;
        fs::write(checkpoint.join("input-request.txt"), question)?;
        return Err("maintainer input is required".into());
    }
    checkpoint_candidate(&workspace, &checkpoint, &response)
}

fn checkpoint_candidate(
    workspace: &Path,
    checkpoint: &Path,
    response: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(checkpoint)?;
    fs::write(
        checkpoint.join("agent-summary.txt"),
        truncate(response.to_owned()),
    )?;
    if git(workspace, &["status", "--porcelain"])?.is_empty() {
        return Err("coding agent made no changes".into());
    }
    git(workspace, &["add", "--all"])?;
    git(
        workspace,
        &[
            "-c",
            "user.name=Ayni Agent",
            "-c",
            "user.email=agent@invalid.example",
            "commit",
            "-s",
            "-m",
            CANDIDATE_COMMIT_TITLE,
        ],
    )?;
    let commit = git(workspace, &["rev-parse", "HEAD"])?;
    let bundle = checkpoint.join("candidate.bundle");
    git(
        workspace,
        &[
            "bundle",
            "create",
            bundle.to_str().ok_or("invalid bundle path")?,
            "HEAD",
        ],
    )?;
    fs::write(
        checkpoint.join("candidate-commit.txt"),
        format!("{commit}\n"),
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    ayni_orchestrator_observability::init("development-agent");
    let process_span = ayni_orchestrator_observability::process_span("development-agent");
    if run().instrument(process_span).await.is_ok() {
        ExitCode::SUCCESS
    } else {
        // Provider and tool errors can contain prompts or repository output.
        error!(
            reason_code = "development_agent_failed",
            "development agent failed"
        );
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_commit_title_is_task_neutral() {
        assert_eq!(
            CANDIDATE_COMMIT_TITLE,
            "chore(agent): apply requested change"
        );
    }

    #[test]
    fn workspace_file_tools_reject_traversal_and_symlinks() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let workspace = Workspace(root.clone());
        assert!(workspace.path("../outside").is_err());
        assert!(workspace.path("/etc/passwd").is_err());
        assert_eq!(workspace.path("AGENTS.md")?, root.join("AGENTS.md"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", root.join("linked"))?;
            assert!(workspace.path("linked").is_err());
        }
        Ok(())
    }
}
