//! Model/tool lifecycle diagnostics, without arguments, responses or credential bindings.
use ayni_orchestrator_observability::workflow::{Workflow, finish};
use rig_agent::agent::{
    AgentHook, HookContext, InvalidToolCallAction, InvalidToolCallContext, ToolCall,
    ToolCallAction, ToolResultAction, ToolResultEvent,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone, Debug, Serialize)]
pub struct ToolObservation {
    pub tool: String,
    pub succeeded: bool,
    pub latency_ms: u64,
}
#[derive(Default)]
struct State {
    pending: HashMap<String, (tracing::Span, Instant)>,
    completed: Vec<ToolObservation>,
}
#[derive(Clone, Default)]
pub struct WorkflowHook {
    pub workflow: Workflow,
    state: Arc<Mutex<State>>,
}
impl WorkflowHook {
    pub(crate) fn new(workflow: Workflow) -> Self {
        Self {
            workflow,
            ..Self::default()
        }
    }
    pub(crate) fn observations(&self) -> Vec<ToolObservation> {
        self.state
            .lock()
            .map_or_else(|_| Vec::new(), |s| s.completed.clone())
    }
}
impl AgentHook for WorkflowHook {
    async fn on_invalid_tool_call(
        &self,
        _: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        let span = self.workflow.span("agent.tool");
        span.record("tool", safe_tool(&event.tool_name));
        finish(&span, false);
        if let Ok(mut state) = self.state.lock()
            && state.completed.len() < 512
        {
            state.completed.push(ToolObservation {
                tool: safe_tool(&event.tool_name).into(),
                succeeded: false,
                latency_ms: 0,
            });
        }
        None
    }

    async fn on_tool_call(&self, _: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        let span = self.workflow.span("agent.tool");
        span.record("tool", safe_tool(event.tool_name));
        if let Ok(mut state) = self.state.lock() {
            // Diagnostic bookkeeping never changes application execution or authority.
            if state.pending.len() < 128 {
                state
                    .pending
                    .insert(event.internal_call_id.into(), (span, Instant::now()));
            }
        }
        ToolCallAction::Run
    }
    async fn on_tool_result(
        &self,
        _: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        if let Ok(mut state) = self.state.lock()
            && let Some((span, started)) = state.pending.remove(event.internal_call_id)
        {
            let succeeded =
                event.raw_result.error().is_none() && event.raw_result.refusal().is_none();
            finish(&span, succeeded);
            if state.completed.len() < 512 {
                state.completed.push(ToolObservation {
                    tool: safe_tool(event.tool_name).into(),
                    succeeded,
                    latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                });
            }
        }
        ToolResultAction::Keep
    }
}
fn safe_tool(name: &str) -> &str {
    use ayni_orchestrator_runtime::agent_profile::{PLANNER_TOOLS, WORKER_TOOLS};
    if PLANNER_TOOLS.contains(&name) || WORKER_TOOLS.contains(&name) {
        name
    } else {
        "unknown_tool"
    }
}
