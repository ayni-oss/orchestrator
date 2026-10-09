//! Model-provider adapter for one bounded coding session.
use super::{
    model_usage::{TrackedModel, UsageTracker},
    recovery,
    sessions::{Context, INSTRUCTIONS},
    tools::{AyniCheck, AyniVerify, CommitChanges, RunCommand},
};
use ayni_orchestrator_domain::ModelUsageStage;
use ayni_orchestrator_runtime::{activity::Stage, runtime_execution::read_bounded};
use rig_agent::{
    agent::{AgentHook, CompletionCallAction, CompletionCallEvent, HookContext},
    client::AgentModelExt,
    completion::Prompt,
};
use rig_core::{client::completion::CompletionClient, providers::openai};
use std::{future::Future, io};

/// Adapter contract preserves frozen limits and durable session accounting.
/// Implementations return a bounded input question, or finish the coding turn.
pub(super) trait CodingAgentRunner {
    fn run(
        &self,
        context: &Context,
        repair: Option<&str>,
    ) -> impl Future<Output = io::Result<Option<String>>> + Send;
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

/// Existing model adapter. Ayni's supported CLI currently owns environment and
/// quality execution; a coding-agent CLI contract must be defined before replacement.
pub(super) struct RigCodingAgent;

impl CodingAgentRunner for RigCodingAgent {
    async fn run(&self, context: &Context, repair: Option<&str>) -> io::Result<Option<String>> {
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
        let client =
            openai::Client::new(std::str::from_utf8(&key).map_err(io::Error::other)?.trim())
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
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
