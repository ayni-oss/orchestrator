//! Coding sessions share one durable turn budget and retain bounded prior context.
use super::{
    activity::ActivityLog,
    model_usage::{TrackedModel, UsageTracker},
    recovery,
    tools::{AyniCheck, AyniVerify, CommitChanges, RunCommand},
};
use ayni_orchestrator_domain::ModelUsageStage;
use ayni_orchestrator_profile_ayni::coding::WorkerSpec;
use ayni_orchestrator_runtime::{
    activity::Stage,
    coding::{PublicationOutcome, WorkerPhase, WorkerProgress},
    runtime_execution::read_bounded,
};
use rig_agent::{
    agent::{AgentHook, CompletionCallAction, CompletionCallEvent, HookContext},
    client::AgentModelExt,
    completion::Prompt,
};
use rig_core::{client::completion::CompletionClient, providers::openai};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const CHECK_RESERVE_SECONDS: u64 = 180;
pub const PUBLICATION_RESERVE_SECONDS: u64 = 90;
pub const INSTRUCTIONS: &str = "Read AGENTS.md and repository instructions. Make only the requested changes within the frozen constraints. Use ayni_verify for focused feedback and ayni_check for full checking. Never weaken .ayni.toml or .ayni.lock to silence findings. Use commit_changes for coherent feature milestones and actual quality repairs, with meaningful Conventional Commit subjects. The worker always runs a mandatory full check after you finish; complete your implementation and explain what changed. Do not access provider APIs, push branches or create PRs; the worker handles delivery. If essential information is missing respond INPUT_REQUIRED: followed by one concrete question.";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    pub progress: WorkerProgress,
    pub prior_conversation: String,
    pub last_response: String,
    pub repair_stopped: bool,
}
impl Default for SessionState {
    fn default() -> Self {
        Self {
            progress: WorkerProgress {
                phase: WorkerPhase::Admission,
                coding_turns_consumed: 0,
                quality_round: 0,
                latest_full_check: None,
                publication: PublicationOutcome::Pending,
            },
            prior_conversation: String::new(),
            last_response: String::new(),
            repair_stopped: false,
        }
    }
}
#[derive(Clone)]
pub struct Context {
    pub spec: Arc<WorkerSpec>,
    pub workspace: PathBuf,
    pub output: PathBuf,
    pub state: Arc<tokio::sync::Mutex<SessionState>>,
    pub serial: Arc<tokio::sync::Mutex<()>>,
    pub activity: ActivityLog,
}
impl Context {
    pub fn remaining(&self) -> io::Result<Duration> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        let deadline = u64::try_from(self.spec.limits.deadline_at).map_err(io::Error::other)?;
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "frozen task deadline exhausted",
            ));
        }
        Ok(Duration::from_secs(deadline - now))
    }
    pub fn coding_time(&self) -> io::Result<Duration> {
        self.remaining()?
            .checked_sub(Duration::from_secs(
                CHECK_RESERVE_SECONDS + PUBLICATION_RESERVE_SECONDS,
            ))
            .filter(|v| !v.is_zero())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "time reserved for final check and publication",
                )
            })
    }
    pub async fn persist(&self) -> io::Result<()> {
        let state = self.state.lock().await;
        recovery::write_json(&self.output.join("progress.json"), &state.progress)?;
        recovery::write_json(&self.output.join("session.json"), &*state)
    }
    pub async fn phase(&self, phase: WorkerPhase) -> io::Result<()> {
        self.state.lock().await.progress.phase = phase;
        self.persist().await
    }
    pub async fn save_source(&self) -> io::Result<()> {
        let context = self.clone();
        let state = context.state.lock().await.clone();
        tokio::task::spawn_blocking(move || {
            recovery::save(&context.spec, &context.workspace, &context.output, &state)
        })
        .await
        .map_err(io::Error::other)?
    }
}
#[derive(Clone)]
struct BudgetHook(Context);
impl AgentHook for BudgetHook {
    async fn on_completion_call(
        &self,
        _: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.0.coding_time().is_err() {
            return CompletionCallAction::stop("time reserved for checking and publication");
        }
        let mut state = self.0.state.lock().await;
        if state.progress.coding_turns_consumed >= self.0.spec.limits.coding_turns {
            return CompletionCallAction::stop("frozen total coding turn budget exhausted");
        }
        state.progress.coding_turns_consumed += 1;
        if let Ok(history) = serde_json::to_string(event.history) {
            state.prior_conversation = tail(&history, 16384);
        }
        if recovery::write_json(&self.0.output.join("session.json"), &*state).is_err()
            || recovery::write_json(&self.0.output.join("progress.json"), &state.progress).is_err()
        {
            return CompletionCallAction::stop("cannot persist consumed turn budget");
        }
        drop(state);
        CompletionCallAction::Continue
    }
}
fn tail(value: &str, bound: usize) -> String {
    value
        .chars()
        .rev()
        .take(bound)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

pub async fn run(context: &Context, repair: Option<&str>) -> io::Result<Option<String>> {
    let state = context.state.lock().await.clone();
    let turns = context
        .spec
        .limits
        .coding_turns
        .saturating_sub(state.progress.coding_turns_consumed);
    if turns == 0 {
        return Ok(None);
    }
    let time = context.coding_time()?;
    context.activity.record(Stage::ModelCredentials);
    let key = read_bounded(std::path::Path::new("/model/api-key"), 16384)?;
    let client = openai::Client::new(std::str::from_utf8(&key).map_err(io::Error::other)?.trim())
        .map_err(io::Error::other)?;
    let usage = UsageTracker::new(
        context.output.join("usage"),
        ModelUsageStage::Coding,
        "openai",
        &context.spec.model,
    )?;
    let agent = TrackedModel::new(client.completion_model(&context.spec.model), usage.clone())
        .into_agent_builder()
        .preamble(INSTRUCTIONS)
        .tool(RunCommand(context.clone()))
        .tool(AyniVerify(context.clone()))
        .tool(AyniCheck(context.clone()))
        .tool(CommitChanges(context.clone()))
        .add_hook(BudgetHook(context.clone()))
        .add_hook(context.activity.clone())
        .build();
    let prompt = format!(
        "Original request: {}\nAcceptance criteria: {}\nFrozen constraints: {}\nPrior bounded conversation: {}\nRepair findings: {}",
        context.spec.request,
        serde_json::to_string(&context.spec.acceptance_criteria)?,
        serde_json::to_string(&context.spec.constraints)?,
        state.prior_conversation,
        repair.unwrap_or("initial implementation")
    );
    context.activity.record(Stage::ModelExecution);
    let response = tokio::time::timeout(
        time,
        agent
            .prompt(prompt)
            .max_turns(usize::from(turns))
            .extended_details(),
    )
    .await;
    match response {
        Ok(Ok(response)) => {
            let mut state = context.state.lock().await;
            state.last_response = response.output.chars().take(8192).collect();
            if let Some(messages) = response.messages {
                state.prior_conversation = tail(&serde_json::to_string(&messages)?, 16384);
            }
            let question = state
                .last_response
                .trim()
                .strip_prefix("INPUT_REQUIRED:")
                .map(|v| v.trim().chars().take(1024).collect::<String>())
                .filter(|v| !v.is_empty());
            drop(state);
            context.persist().await?;
            Ok(question)
        }
        Ok(Err(error)) => {
            context
                .activity
                .fail(super::activity::prompt_failure(&error));
            usage.fail_pending(None)?;
            // Exhausted turns do not skip the coordinator's mandatory final check.
            if matches!(
                error,
                rig_agent::completion::PromptError::MaxTurnsError { .. }
                    | rig_agent::completion::PromptError::PromptCancelled { .. }
            ) {
                Ok(None)
            } else {
                Err(io::Error::other("coding model execution unavailable"))
            }
        }
        Err(_) => {
            usage.fail_pending(None)?;
            Ok(None)
        }
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
