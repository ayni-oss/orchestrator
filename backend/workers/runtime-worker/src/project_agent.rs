//! Read-only repository research. The model never receives a process or write tool.
use ayni_orchestrator_runtime::{
    project_agent::{ProjectAgentRequest, ProjectAgentResult, ResearchProposal, research_key},
    runtime_execution::{ExecutionRequest, read_bounded},
};
use rig_agent::{
    agent::{AgentHook, CompletionCallAction, CompletionCallEvent, HookContext},
    client::AgentModelExt,
    completion::Prompt,
    tool::{Tool, ToolContext},
};
use rig_core::client::completion::CompletionClient;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write as _;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

#[path = "project_context.rs"]
mod conversation_context;

use ayni_orchestrator_runtime::agent_profile::PLANNER_INSTRUCTIONS as PREAMBLE;

#[derive(Clone)]
struct Context {
    request: Arc<ProjectAgentRequest>,
    root: PathBuf,
    proposals: Arc<tokio::sync::Mutex<Vec<ResearchProposal>>>,
}
impl Context {
    fn authorize(&self, tool: &str, repository: Option<&str>) -> io::Result<()> {
        if let Some(profile) = &self.request.profile {
            profile.authorize(tool, repository)?;
        }
        Ok(())
    }
    fn repository(&self, id: &str) -> io::Result<PathBuf> {
        if !self.request.repositories.iter().any(|r| r.id == id) {
            return Err(io::Error::other("repository outside research scope"));
        }
        let root = self.root.join(research_key(id));
        if fs::symlink_metadata(&root)?.file_type().is_symlink() {
            return Err(io::Error::other("symlink denied"));
        }
        Ok(root)
    }
}
fn bounded_text(value: &str, maximum: usize) -> String {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}
fn safe_path(root: &Path, relative: &str) -> io::Result<PathBuf> {
    if relative.len() > 512 {
        return Err(io::Error::other("path exceeds bound"));
    }
    let mut result = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::other("path must be repository relative"));
        };
        if name == ".git" {
            return Err(io::Error::other("git metadata excluded"));
        }
        result.push(name);
        if fs::symlink_metadata(&result)?.file_type().is_symlink() {
            return Err(io::Error::other("symlink denied"));
        }
    }
    Ok(result)
}
fn files(root: &Path) -> io::Result<Vec<String>> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    let mut visited = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            visited += 1;
            if visited > 10000 {
                return Err(io::Error::other("repository listing exceeds bound"));
            }
            let entry = entry?;
            if entry.file_name() == ".git" {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                paths.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(io::Error::other)?
                        .to_string_lossy()
                        .into_owned(),
                );
                if paths.len() > 4096 {
                    return Err(io::Error::other("repository file count exceeds bound"));
                }
            }
        }
    }
    paths.sort();
    Ok(paths)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoArgs {
    repository_id: String,
}
#[derive(Clone)]
struct ListFiles(Context);
impl Tool for ListFiles {
    const NAME: &'static str = "list_repository_files";
    type Args = RepoArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "List bounded ordinary files in an immutable repository, excluding Git metadata and symlinks.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"repository_id":{"type":"string"}},"required":["repository_id"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: RepoArgs) -> io::Result<String> {
        self.0.authorize(Self::NAME, Some(&args.repository_id))?;
        let result = files(&self.0.repository(&args.repository_id)?)?.join("\n");
        Ok(bounded_text(&result, 16384))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    repository_id: String,
    path: String,
}
#[derive(Clone)]
struct ReadFile(Context);
impl Tool for ReadFile {
    const NAME: &'static str = "read_repository_file";
    type Args = ReadArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Read a bounded UTF-8 file. Absolute paths, traversal, symlinks and Git metadata are denied.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"repository_id":{"type":"string"},"path":{"type":"string"}},"required":["repository_id","path"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: ReadArgs) -> io::Result<String> {
        self.0.authorize(Self::NAME, Some(&args.repository_id))?;
        let root = self.0.repository(&args.repository_id)?;
        String::from_utf8(read_bounded(&safe_path(&root, &args.path)?, 32768)?)
            .map_err(io::Error::other)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    repository_id: String,
    text: String,
}
#[derive(Clone)]
struct SearchFiles(Context);
impl Tool for SearchFiles {
    const NAME: &'static str = "search_repository";
    type Args = SearchArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Search literal text in bounded ordinary source files. Returns paths and line numbers."
            .into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"repository_id":{"type":"string"},"text":{"type":"string"}},"required":["repository_id","text"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: SearchArgs) -> io::Result<String> {
        self.0.authorize(Self::NAME, Some(&args.repository_id))?;
        if args.text.is_empty() || args.text.len() > 256 {
            return Err(io::Error::other("invalid search text"));
        }
        let root = self.0.repository(&args.repository_id)?;
        let mut output = String::new();
        for path in files(&root)? {
            let Ok(bytes) = read_bounded(&safe_path(&root, &path)?, 65536) else {
                continue;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                if line.contains(&args.text) {
                    writeln!(
                        output,
                        "{path}:{}:{}",
                        index + 1,
                        line.chars().take(512).collect::<String>()
                    )
                    .map_err(io::Error::other)?;
                    if output.len() > 16000 {
                        return Ok(bounded_text(&output, 16384));
                    }
                }
            }
        }
        Ok(output)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}
#[derive(Clone)]
struct ProjectContext(Context);
impl Tool for ProjectContext {
    const NAME: &'static str = "inspect_project_context";
    type Args = EmptyArgs;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Read trusted repository identities, runtime readiness, available model choices, tasks, attempt outcomes and publication receipts.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{},"required":[],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, _: EmptyArgs) -> io::Result<String> {
        self.0.authorize(Self::NAME, None)?;
        Ok(self.0.request.context.clone())
    }
}
#[derive(Clone)]
struct PrepareProposal(Context);
impl Tool for PrepareProposal {
    const NAME: &'static str = "prepare_task_proposal";
    type Args = ResearchProposal;
    type Output = String;
    type Error = io::Error;
    fn description(&self) -> String {
        "Prepare a complete task or an explicitly requested lifecycle action for human approval. This never executes the proposed action.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"findings":{"type":"string"},"scope":{"type":"string"},"dependency_environment_impact":{"type":"string"},"repository_id":{"type":"string"},"revision":{"type":"string"},"runtime_profile_id":{"type":"string"},"coding_connection_id":{"type":"string"},"coding_model":{"type":"string"},"request":{"type":"string"},"acceptance_criteria":{"type":"array","items":{"type":"string"}},"allowed_paths":{"type":"array","items":{"type":"string"}},"allow_test_edits":{"type":"boolean"},"forbidden_paths":{"type":"array","items":{"type":"string"}},"coding_turns":{"type":"integer","minimum":1,"maximum":128},"deadline_seconds":{"type":"integer","minimum":1,"maximum":86400},"action":{"type":"string","enum":["create_task","cancel_task","retry_task","retry_publication"]},"target_task_id":{"type":"string"}},"required":["findings","scope","dependency_environment_impact","repository_id","revision","runtime_profile_id","coding_connection_id","coding_model","request","acceptance_criteria","allowed_paths","allow_test_edits","forbidden_paths","coding_turns","deadline_seconds","action","target_task_id"],"additionalProperties":false})
    }
    async fn call(&self, _: &mut ToolContext, args: ResearchProposal) -> io::Result<String> {
        self.0.authorize(Self::NAME, Some(&args.repository_id))?;
        let repo = self
            .0
            .request
            .repositories
            .iter()
            .find(|r| r.id == args.repository_id)
            .ok_or_else(|| io::Error::other("repository outside research scope"))?;
        if args.revision != repo.revision {
            return Err(io::Error::other(
                "proposal source differs from researched source",
            ));
        }
        if args.action != "create_task" {
            let last = self
                .0
                .request
                .history
                .rsplit("user:")
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if !ayni_orchestrator_runtime::project_agent::explicit_action_request(
                &last,
                &args.action,
                &args.target_task_id,
            ) {
                return Err(io::Error::other(
                    "lifecycle action requires explicit user request naming the task",
                ));
            }
        }
        if args.findings.len() > 16384
            || args.scope.len() > 16384
            || args.dependency_environment_impact.len() > 16384
            || args.request.trim().is_empty()
            || args.request.len() > 16384
            || args.acceptance_criteria.is_empty()
            || args.acceptance_criteria.len() > 20
            || args.coding_model.is_empty()
            || (args.action == "create_task" && args.coding_connection_id.is_empty())
            || args.coding_turns == 0
            || args.coding_turns > 128
            || args.deadline_seconds == 0
            || args.deadline_seconds > 86400
        {
            return Err(io::Error::other("incomplete or oversized proposal"));
        }
        if let Some(profile) = &self.0.request.worker_profile
            && (args.coding_connection_id != profile.connection_id
                || args.coding_model != profile.model
                || args.coding_turns > profile.settings.max_model_calls
                || args.deadline_seconds > profile.settings.deadline_seconds)
        {
            return Err(io::Error::other("proposal exceeds frozen worker profile"));
        }
        let mut proposals = self.0.proposals.lock().await;
        if proposals
            .iter()
            .any(|p| p.repository_id == args.repository_id)
        {
            return Err(io::Error::other("only one proposal per repository per run"));
        }
        proposals.push(args);
        drop(proposals);
        Ok(
            "Proposal prepared. The user must approve the exact proposal before any action occurs."
                .into(),
        )
    }
}
#[derive(Clone)]
struct Budget {
    calls: Arc<AtomicU32>,
    limit: u32,
}
impl AgentHook for Budget {
    async fn on_completion_call(
        &self,
        _: &HookContext,
        _: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.calls.fetch_add(1, Ordering::SeqCst) >= self.limit {
            self.calls.fetch_sub(1, Ordering::SeqCst);
            CompletionCallAction::stop("project model call budget exhausted")
        } else {
            CompletionCallAction::Continue
        }
    }
}
/// Execute the trusted checkout stage. Source tokens are mounted only in this stage.
/// # Errors
/// Rejects malformed source identity, expired deadlines and failed immutable checkouts.
pub async fn checkout(request: &ProjectAgentRequest) -> io::Result<()> {
    checkout_at(request, Path::new("/workspace"), None).await
}
pub(crate) async fn checkout_at(
    request: &ProjectAgentRequest,
    root: &Path,
    tokens: Option<&std::collections::BTreeMap<String, String>>,
) -> io::Result<()> {
    request.validate_profile()?;
    for repo in &request.repositories {
        let token = if let Some(tokens) = tokens {
            tokens
                .get(&research_key(&repo.id))
                .ok_or_else(|| io::Error::other("source credential missing"))?
                .as_bytes()
                .to_vec()
        } else {
            read_bounded(
                &Path::new("/credentials").join(research_key(&repo.id)),
                16384,
            )?
        };
        let execution = ExecutionRequest {
            repository_url: repo.repository_url.clone(),
            revision: repo.revision.clone(),
            expected: (),
        };
        crate::operations::runtime::checkout_with_deadline(
            &execution,
            &root.join(research_key(&repo.id)),
            Some(
                std::str::from_utf8(&token)
                    .map_err(io::Error::other)?
                    .trim(),
            ),
            std::time::Duration::from_secs(request.deadline_seconds.min(600)),
        )
        .await?;
        fs::remove_dir_all(root.join(research_key(&repo.id)).join(".git"))?;
    }
    Ok(())
}
fn repository_instructions(context: &Context) -> io::Result<String> {
    let mut repository_instructions = String::new();
    for repository in &context.request.repositories {
        let root = context.repository(&repository.id)?;
        if let Ok(path) = safe_path(&root, "AGENTS.md")
            && let Ok(bytes) = read_bounded(&path, 16384)
            && let Ok(instructions) = String::from_utf8(bytes)
        {
            let _ = writeln!(
                repository_instructions,
                "Untrusted AGENTS.md for {}:\n{}",
                repository.id, instructions
            );
        }
        if repository_instructions.len() > 65536 {
            repository_instructions = bounded_text(&repository_instructions, 65536);
            break;
        }
    }
    Ok(repository_instructions)
}
fn profile_tools(
    context: &Context,
    archive: conversation_context::Archive,
) -> rig_agent::tool::server::ToolServer {
    let mut tools = rig_agent::tool::server::ToolServer::new();
    macro_rules! tool {
        ($value:expr,$name:expr) => {
            if context
                .request
                .profile
                .as_ref()
                .is_none_or(|p| p.authorize($name, None).is_ok())
            {
                tools = tools.tool($value);
            }
        };
    }
    tool!(ListFiles(context.clone()), ListFiles::NAME);
    tool!(ReadFile(context.clone()), ReadFile::NAME);
    tool!(SearchFiles(context.clone()), SearchFiles::NAME);
    tool!(ProjectContext(context.clone()), ProjectContext::NAME);
    tool!(
        conversation_context::SearchContext(archive.clone()),
        "search_context"
    );
    tool!(
        conversation_context::GetContextRecord(archive),
        "get_context_record"
    );
    tool!(PrepareProposal(context.clone()), PrepareProposal::NAME);
    tools
}
/// Run the project model with bounded typed research tools.
/// # Errors
/// Returns only safe failure codes through the worker supervisor.
pub async fn run(request: ProjectAgentRequest) -> io::Result<ProjectAgentResult> {
    run_at(request, Path::new("/workspace")).await
}
pub(crate) async fn run_at(
    request: ProjectAgentRequest,
    root: &Path,
) -> io::Result<ProjectAgentResult> {
    if request.max_model_calls == 0
        || request.max_model_calls > 128
        || request.deadline_seconds == 0
        || request.deadline_seconds > 86400
        || request.context.len() > 262_144
        || request.history.len() > 131_072
    {
        return Err(io::Error::other("project request bounds invalid"));
    }
    request.validate_profile()?;
    conversation_context::Archive::from_request(&request)?;
    let key = read_bounded(Path::new("/model/api-key"), 16384)?;
    let client = ayni_orchestrator_providers::openai_client(
        "openai",
        std::str::from_utf8(&key).map_err(io::Error::other)?.trim(),
    )
    .map_err(io::Error::other)?;
    let model = client.completion_model(&request.model);
    let hook = crate::workflow_trace::WorkflowHook::new(workflow(&request));
    run_with_model(request, root, model, hook).await
}

pub(crate) fn workflow(
    request: &ProjectAgentRequest,
) -> ayni_orchestrator_observability::workflow::Workflow {
    ayni_orchestrator_observability::workflow::Workflow {
        project_id: request.project_id.clone(),
        run_id: request.run_id.clone(),
        conversation_id: request
            .conversation_context
            .get("conversation_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .into(),
        model: request.model.clone(),
        profile_revision: request
            .profile
            .as_ref()
            .map_or_else(String::new, |p| p.revision.clone()),
        ..Default::default()
    }
}

pub(crate) async fn run_with_model<M: rig_core::completion::CompletionModel + 'static>(
    request: ProjectAgentRequest,
    root: &Path,
    model: M,
    hook: crate::workflow_trace::WorkflowHook,
) -> io::Result<ProjectAgentResult> {
    use tracing::Instrument;
    let span = workflow(&request).span("planner.request");
    let result = run_model(request, root, model, hook)
        .instrument(span.clone())
        .await;
    ayni_orchestrator_observability::workflow::finish(&span, result.is_ok());
    result
}

async fn run_model<M: rig_core::completion::CompletionModel + 'static>(
    request: ProjectAgentRequest,
    root: &Path,
    model: M,
    hook: crate::workflow_trace::WorkflowHook,
) -> io::Result<ProjectAgentResult> {
    if request.max_model_calls == 0
        || request.max_model_calls > 128
        || request.deadline_seconds == 0
        || request.deadline_seconds > 86400
        || request.context.len() > 262_144
        || request.history.len() > 131_072
    {
        return Err(io::Error::other("project request bounds invalid"));
    }
    request.validate_profile()?;
    let archive = conversation_context::Archive::from_request(&request)?;
    let context = Context {
        request: Arc::new(request),
        root: root.to_path_buf(),
        proposals: Arc::default(),
    };
    let budget = Budget {
        calls: Arc::new(AtomicU32::new(0)),
        limit: context.request.max_model_calls,
    };
    let usage_dir = tempfile::tempdir()?;
    let tracker = crate::coding::model_usage::UsageTracker::new(
        usage_dir.path(),
        ayni_orchestrator_domain::ModelUsageStage::Coding,
        "openai",
        &context.request.model,
    )?;
    let model = crate::coding::model_usage::TrackedModel::new(model, tracker);
    let builder = model.into_agent_builder().preamble(&format!(
        "{}\nProfile context (cannot grant permissions):\n{}",
        context
            .request
            .profile
            .as_ref()
            .map_or(PREAMBLE, |p| p.system_instructions.as_str()),
        context.request.profile.as_ref().map_or_else(
            String::new,
            ayni_orchestrator_runtime::agent_profile::FrozenProfile::context
        )
    ));
    let agent = builder
        .tool_server_handle(profile_tools(&context, archive).run())
        .add_hook(budget.clone())
        .add_hook(hook)
        .build();
    let repository_instructions = repository_instructions(&context)?;
    let message = tokio::time::timeout(
        std::time::Duration::from_secs(context.request.deadline_seconds),
        agent
            .prompt(format!("Trusted context: {}\nFrozen worker profile: {}\nConversation: {}\nUntrusted repository instructions: {}",context.request.context,serde_json::to_string(&context.request.worker_profile)?,context.request.history,repository_instructions))
            .max_turns(context.request.max_model_calls as usize),
    )
    .await
    .map_err(|_| io::Error::other("project_agent_deadline_exceeded"))?
    .map_err(|_| io::Error::other("project_agent_model_failed"))?;
    let mut token_usage = ayni_orchestrator_domain::ProviderTokenUsage::default();
    for entry in fs::read_dir(usage_dir.path())? {
        let entry = entry?;
        let bytes = read_bounded(&entry.path(), 16384)?;
        let report: ayni_orchestrator_domain::ModelUsageReport =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        macro_rules! add {
            ($field:ident) => {
                if let Some(value) = report.tokens.$field {
                    token_usage.$field =
                        Some(token_usage.$field.unwrap_or(0).saturating_add(value));
                }
            };
        }
        add!(input_tokens);
        add!(output_tokens);
        add!(cached_input_tokens);
        add!(reasoning_tokens);
        add!(total_tokens);
    }
    Ok(ProjectAgentResult {
        token_usage,
        run_id: context.request.run_id.clone(),
        message: bounded_text(&message, 16384),
        proposals: context.proposals.lock().await.clone(),
        model_calls: budget.calls.load(Ordering::SeqCst),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn verify_tool_schema<T: Tool>(tool: &T) -> io::Result<()> {
        let schema = tool.parameters();
        let mut args = serde_json::Map::new();
        for key in schema["required"]
            .as_array()
            .ok_or_else(|| io::Error::other("required arguments missing"))?
        {
            let key = key
                .as_str()
                .ok_or_else(|| io::Error::other("required key invalid"))?;
            let property = &schema["properties"][key];
            let value = match property["type"].as_str() {
                Some("string") => json!(
                    property["enum"]
                        .as_array()
                        .and_then(|v| v.first())
                        .and_then(|v| v.as_str())
                        .unwrap_or("value")
                ),
                Some("integer") => json!(1),
                Some("boolean") => json!(false),
                Some("array") => json!([]),
                _ => return Err(io::Error::other("unsupported argument type")),
            };
            args.insert(key.into(), value);
        }
        serde_json::from_value::<T::Args>(serde_json::Value::Object(args))
            .map_err(io::Error::other)?;
        Ok(())
    }
    #[test]
    fn advertised_research_tool_arguments_match_deserialization_contracts() -> io::Result<()> {
        let context = Context {
            request: Arc::new(ProjectAgentRequest {
                worker_profile: None,
                profile: None,
                run_id: "run".into(),
                project_id: "project".into(),
                model: "model".into(),
                history: String::new(),
                context: String::new(),
                conversation_context: serde_json::Value::Null,
                repositories: vec![],
                max_model_calls: 24,
                deadline_seconds: 600,
            }),
            root: PathBuf::from("/workspace"),
            proposals: Arc::default(),
        };
        verify_tool_schema(&ListFiles(context.clone()))?;
        verify_tool_schema(&ReadFile(context.clone()))?;
        verify_tool_schema(&SearchFiles(context.clone()))?;
        verify_tool_schema(&ProjectContext(context.clone()))?;
        let archive = conversation_context::Archive::from_request(&context.request)?;
        verify_tool_schema(&conversation_context::SearchContext(archive.clone()))?;
        verify_tool_schema(&conversation_context::GetContextRecord(archive))?;
        verify_tool_schema(&PrepareProposal(context))?;
        Ok(())
    }
    #[test]
    fn logical_repository_ids_use_safe_storage_keys() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let id = "repository:0195e327-5f07-7000-aaaa-ccccddddffff";
        let key = research_key(id);
        assert!(!key.contains(':'));
        fs::create_dir(directory.path().join(&key))?;
        let request = ProjectAgentRequest {
            worker_profile: None,
            profile: None,
            run_id: "run".into(),
            project_id: "project".into(),
            model: "model".into(),
            history: String::new(),
            context: String::new(),
            conversation_context: serde_json::Value::Null,
            max_model_calls: 24,
            deadline_seconds: 600,
            repositories: vec![
                ayni_orchestrator_runtime::project_agent::ResearchRepository {
                    id: id.into(),
                    repository_url: "https://github.com/ayni-oss/sandbox.git".into(),
                    revision: "a".repeat(40),
                    coding_connection_id: String::new(),
                    coding_model: "model".into(),
                    runtime_profile_id: String::new(),
                },
            ],
        };
        let context = Context {
            request: Arc::new(request),
            root: directory.path().into(),
            proposals: Arc::default(),
        };
        assert_eq!(context.repository(id)?, directory.path().join(key));
        assert!(context.repository("repository:other").is_err());
        Ok(())
    }
    #[test]
    fn research_rejects_escape_paths_and_git_metadata() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("source.rs"), "example")?;
        assert!(safe_path(dir.path(), "../source.rs").is_err());
        assert!(safe_path(dir.path(), "/etc/passwd").is_err());
        assert!(safe_path(dir.path(), ".git/config").is_err());
        assert!(safe_path(dir.path(), "source.rs").is_ok());
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn research_rejects_symlinks_and_skips_them_in_listing() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        std::os::unix::fs::symlink("/etc", dir.path().join("escape"))?;
        assert!(safe_path(dir.path(), "escape/passwd").is_err());
        assert!(files(dir.path())?.is_empty());
        Ok(())
    }
}

#[cfg(test)]
#[path = "project_profile_tests.rs"]
mod profile_tests;
