//! Typed quality and milestone tools, with direct bounded process invocation.
use super::{recovery, sessions::Context};
use crate::operations::sandbox::command;
use ayni_orchestrator_runtime::coding::QualityResult;
use rig_agent::tool::{Tool, ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{io, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

#[derive(Clone)]
pub struct RunCommand(pub Context);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandArgs {
    command: String,
}
impl Tool for RunCommand {
    const NAME: &'static str = "run_command";
    type Args = CommandArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Inspect and edit repository files in /workspace. Use commit_changes for coherent milestones. Commands have a bounded deadline and output.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: CommandArgs) -> io::Result<String> {
        let guard = self.0.serial.clone().lock_owned().await;
        if args.command.is_empty() || args.command.len() > 16384 {
            return Err(io::Error::other("command exceeds bound"));
        }
        let deadline = self.0.coding_time()?.min(Duration::from_secs(180));
        let mut process = tokio::process::Command::from(command("/bin/sh", &self.0.workspace)?);
        process
            .args(["-c", &args.command])
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
        let result = tokio::time::timeout(deadline, async {
            tokio::join!(child.wait(), bounded_output(stdout), bounded_output(stderr))
        })
        .await;
        drop(child);
        let state = self.0.state.lock().await.clone();
        let context = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            recovery::save(&context.spec, &context.workspace, &context.output, &state)
        })
        .await
        .map_err(io::Error::other)??;
        let (status, stdout, stderr) = result
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "command deadline exceeded"))?;
        Ok(format!(
            "exit={:?}\nstdout:\n{}\nstderr:\n{}",
            status?.code(),
            stdout?,
            stderr?
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Test,
    Coverage,
    Size,
    Complexity,
    Deps,
    Mutation,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    Go,
    Node,
    Python,
    Kotlin,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyArgs {
    pub signal: Signal,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub file: Option<String>,
    pub package: Option<String>,
    pub name: Option<String>,
}
impl VerifyArgs {
    pub fn arguments(&self) -> io::Result<Vec<String>> {
        let unsupported = match self.signal {
            Signal::Test => false,
            Signal::Size => self.package.is_some() || self.name.is_some(),
            Signal::Complexity | Signal::Deps => self.name.is_some(),
            Signal::Coverage | Signal::Mutation => {
                self.file.is_some() || self.package.is_some() || self.name.is_some()
            }
        };
        if unsupported {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unsupported verify selectors: test accepts file/package/name; size accepts file; complexity/deps accept file/package; coverage/mutation accept only language/root",
            ));
        }
        let token = |value: &serde_json::Value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("invalid selector"))
        };
        let mut args = vec![
            "verify".into(),
            token(&serde_json::to_value(&self.signal)?)?,
        ];
        if let Some(language) = &self.language {
            args.extend([
                "--language".into(),
                token(&serde_json::to_value(language)?)?,
            ]);
        }
        for (flag, value) in [
            ("--root", &self.root),
            ("--file", &self.file),
            ("--package", &self.package),
            ("--name", &self.name),
        ] {
            if let Some(value) = value {
                if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
                    return Err(io::Error::other("selector exceeds bounds"));
                }
                args.extend([flag.into(), value.clone()]);
            }
        }
        Ok(args)
    }
}
#[derive(Clone)]
pub struct AyniVerify(pub Context);
impl Tool for AyniVerify {
    const NAME: &'static str = "ayni_verify";
    type Args = VerifyArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Run ayni verify <signal> with optional language/root/file/package/name selectors. Ayni validates adapter-specific combinations. Focused evidence is feedback, never the full quality gate.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"signal":{"type":"string","enum":["test","coverage","size","complexity","deps","mutation"]},"language":{"type":"string","enum":["rust","go","node","python","kotlin"]},"root":{"type":"string"},"file":{"type":"string","description":"Only test, size, complexity, or deps."},"package":{"type":"string","description":"Only test, complexity, or deps."},"name":{"type":"string","description":"Only test."}},"required":["signal"],"additionalProperties":false})
    }
    fn map_error(&self, error: io::Error) -> rig_agent::tool::ToolExecutionError {
        if error.kind() == io::ErrorKind::InvalidInput {
            rig_agent::tool::ToolExecutionError::invalid_args(error.to_string())
        } else {
            rig_agent::tool::ToolExecutionError::from_error(error)
        }
    }
    async fn call(&self, _: &mut ToolContext, args: VerifyArgs) -> io::Result<String> {
        let _guard = self.0.serial.lock().await;
        let report = crate::profiles::ayni::invoke(
            &self.0.spec.execution,
            &self.0.workspace,
            &args.arguments()?,
            false,
            self.0.coding_time()?,
        )
        .await?;
        summary(&report.quality)
    }
}
#[derive(Clone)]
pub struct AyniCheck(pub Context);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckArgs {}
impl Tool for AyniCheck {
    const NAME: &'static str = "ayni_check";
    type Args = CheckArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Run ayni check for the full repository contract, returning fresh completion accounting and bounded actionable findings. The worker also checks automatically at the final boundary.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, _: CheckArgs) -> io::Result<String> {
        let _guard = self.0.serial.lock().await;
        let report = crate::profiles::ayni::invoke(
            &self.0.spec.execution,
            &self.0.workspace,
            &["check".into()],
            true,
            self.0.coding_time()?,
        )
        .await?;
        let result = summary(&report.quality)?;
        self.0.state.lock().await.progress.latest_full_check = Some(report.quality);
        self.0.persist().await?;
        Ok(result)
    }
}
#[derive(Clone)]
pub struct CommitChanges(pub Context);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitArgs {
    message: String,
}
impl Tool for CommitChanges {
    const NAME: &'static str = "commit_changes";
    type Args = CommitArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Commit a coherent completed feature or repair milestone. Requires a Conventional Commit subject and enforces frozen constraints. Skips empty commits.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: CommitArgs) -> io::Result<String> {
        let guard = self.0.serial.clone().lock_owned().await;
        self.0.coding_time()?;
        let context = self.0.clone();
        let state = context.state.lock().await.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let commit =
                super::commits::commit_changes(&context.spec, &context.workspace, &args.message)?;
            recovery::save(&context.spec, &context.workspace, &context.output, &state)?;
            Ok(commit.unwrap_or_else(|| "No uncommitted changes".into()))
        })
        .await
        .map_err(io::Error::other)?
    }
}
pub fn summary(result: &QualityResult) -> io::Result<String> {
    serde_json::to_string(result).map_err(io::Error::other)
}
async fn bounded_output(mut stream: impl tokio::io::AsyncRead + Unpin) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let keep = read.min(32_000_usize.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn model_json_dispatch_reaches_check_runner_and_rejects_bad_selectors()
    -> Result<(), Box<dyn std::error::Error>> {
        use rig_agent::tool::{ToolErrorKind, ToolSet};
        use std::sync::Arc;
        let root = tempfile::tempdir()?;
        let workspace = root.path().join("workspace");
        let context = Context {
            spec: Arc::new(super::super::tests::fixture(&workspace)?),
            workspace,
            output: root.path().join("output"),
            state: Arc::new(tokio::sync::Mutex::new(
                crate::coding::sessions::SessionState::default(),
            )),
            serial: Arc::new(tokio::sync::Mutex::new(())),
            activity: crate::coding::activity::ActivityLog::default(),
        };
        let mut tools = ToolSet::default();
        tools.add_tool(AyniCheck(context.clone()));
        tools.add_tool(AyniVerify(context));
        let mut dispatch = ToolContext::new();
        // No managed runtime exists in this fixture: valid arguments must reach
        // preflight and fail there, rather than failing JSON decoding.
        let check = tools.execute("ayni_check", "{}", &mut dispatch).await;
        assert_eq!(
            check.error().map(rig_agent::tool::ToolExecutionError::kind),
            Some(ToolErrorKind::Other)
        );
        let extra = tools
            .execute("ayni_check", r#"{"unexpected":true}"#, &mut dispatch)
            .await;
        assert_eq!(
            extra.error().map(rig_agent::tool::ToolExecutionError::kind),
            Some(ToolErrorKind::InvalidArgs)
        );
        for signal in ["test", "coverage", "size", "complexity", "deps", "mutation"] {
            let result = tools
                .execute(
                    "ayni_verify",
                    serde_json::json!({"signal":signal}).to_string(),
                    &mut dispatch,
                )
                .await;
            assert_eq!(
                result
                    .error()
                    .map(rig_agent::tool::ToolExecutionError::kind),
                Some(ToolErrorKind::Other)
            );
        }
        for args in [
            serde_json::json!({"signal":"coverage","file":"src/lib.rs"}),
            serde_json::json!({"signal":"mutation","package":"crate"}),
            serde_json::json!({"signal":"size","package":"crate"}),
            serde_json::json!({"signal":"deps","name":"test"}),
        ] {
            let result = tools
                .execute("ayni_verify", args.to_string(), &mut dispatch)
                .await;
            assert_eq!(
                result
                    .error()
                    .map(rig_agent::tool::ToolExecutionError::kind),
                Some(ToolErrorKind::InvalidArgs)
            );
        }
        Ok(())
    }

    #[test]
    fn selectors_are_arguments_not_shell_fragments() -> io::Result<()> {
        let args = VerifyArgs {
            signal: Signal::Test,
            language: Some(Language::Rust),
            root: Some(".".into()),
            file: None,
            package: Some("crate; echo nope".into()),
            name: Some("name with spaces".into()),
        }
        .arguments()?;
        assert!(args.contains(&"crate; echo nope".into()));
        assert!(args.contains(&"name with spaces".into()));
        Ok(())
    }
    #[tokio::test]
    async fn output_is_drained_after_the_return_bound() -> io::Result<()> {
        assert_eq!(
            bounded_output(vec![b'a'; 100_000].as_slice()).await?.len(),
            32_000
        );
        Ok(())
    }
}
