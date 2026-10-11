//! Rig lifecycle observer that never retains model or tool content.
use ayni_orchestrator_runtime::activity::{Activity, Failure, MAX_EVENTS, Stage};
use rig_agent::agent::{
    AgentHook, CompletionCallAction, CompletionCallEvent, CompletionResponseEvent, HookContext,
    ObservationAction, ToolCall, ToolCallAction, ToolResultAction, ToolResultEvent,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct ActivityLog(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    path: Option<PathBuf>,
    events: Vec<Activity>,
    sequence: u32,
    stage: Option<Stage>,
}

impl ActivityLog {
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        Self(Arc::new(Mutex::new(State {
            path,
            ..State::default()
        })))
    }

    pub fn record(&self, stage: Stage) {
        self.emit(Some(stage), None);
    }

    pub fn fail(&self, failure: Failure) {
        self.emit(None, Some(failure));
    }

    fn emit(&self, stage: Option<Stage>, failure: Option<Failure>) {
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        if failure.is_some() && log.events.last().is_some_and(|event| event.failed) {
            return;
        }
        let failed = failure.is_some();
        let stage = stage.or(log.stage).unwrap_or(Stage::Request);
        log.stage = Some(stage);
        log.sequence = log.sequence.saturating_add(1);
        let event = Activity {
            sequence: log.sequence,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|time| i64::try_from(time.as_millis()).ok())
                .unwrap_or(0),
            stage,
            failed,
            failure,
        };
        if failed {
            tracing::error!(stage = ?stage, failure = ?failure, sequence = event.sequence, "worker stage failed");
        } else {
            tracing::info!(stage = ?stage, sequence = event.sequence, message = stage.message());
        }
        if log.events.len() == MAX_EVENTS {
            log.events.remove(0);
        }
        log.events.push(event);
        if let Some(path) = &log.path {
            let result = serde_json::to_vec(&log.events)
                .map_err(std::io::Error::other)
                .and_then(|bytes| fs::write(path.with_extension("pending"), bytes))
                .and_then(|()| fs::rename(path.with_extension("pending"), path));
            if result.is_err() {
                tracing::warn!("activity persistence unavailable; use worker logs");
            }
        }
    }
}

impl AgentHook for ActivityLog {
    async fn on_completion_call(
        &self,
        _: &HookContext,
        _: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        self.record(Stage::ModelRequest);
        CompletionCallAction::Continue
    }
    async fn on_completion_response(
        &self,
        _: &HookContext,
        _: CompletionResponseEvent<'_>,
    ) -> ObservationAction {
        self.record(Stage::ModelResponse);
        ObservationAction::Continue
    }
    async fn on_tool_call(&self, _: &HookContext, _: ToolCall<'_>) -> ToolCallAction {
        self.record(Stage::ToolStarted);
        ToolCallAction::Run
    }
    async fn on_tool_result(
        &self,
        _: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        self.record(Stage::ToolFinished);
        self.record_tool_failure(event.raw_result);
        ToolResultAction::Keep
    }
}

impl ActivityLog {
    fn record_tool_failure(&self, result: &rig_agent::tool::ToolResult) {
        use rig_agent::tool::ToolErrorKind;
        if let Some(error) = result.error().or_else(|| result.refusal()) {
            self.fail(match error.kind() {
                ToolErrorKind::InvalidArgs => Failure::InvalidInput,
                ToolErrorKind::Cancelled => Failure::Cancelled,
                ToolErrorKind::NotFound => Failure::MissingFile,
                ToolErrorKind::PermissionDenied => Failure::PermissionDenied,
                ToolErrorKind::RateLimited => Failure::RateLimited,
                ToolErrorKind::Network => Failure::Transport,
                _ => Failure::Internal,
            });
        }
    }
}

/// Classify Rig errors without formatting bodies, histories, URLs, or tool arguments.
#[must_use]
pub fn prompt_failure(error: &rig_agent::completion::PromptError) -> Failure {
    use rig_agent::completion::PromptError;
    use rig_core::completion::CompletionError;
    match error {
        PromptError::CompletionError(error) => {
            match error
                .provider_response_status()
                .map(|status| status.as_u16())
            {
                Some(401 | 403) => return Failure::Authentication,
                Some(429) => return Failure::RateLimited,
                Some(500..=599) => return Failure::ProviderUnavailable,
                Some(400..=499) => return Failure::ProviderRejected,
                _ => {}
            }
            match error {
                CompletionError::HttpError(_) => Failure::Transport,
                CompletionError::JsonError(_) | CompletionError::ResponseError(_) => {
                    Failure::InvalidResponse
                }
                CompletionError::UrlError(_) | CompletionError::RequestError(_) => {
                    Failure::InvalidRequest
                }
                CompletionError::ProviderError(_) | CompletionError::ProviderResponse(_) => {
                    Failure::ProviderRejected
                }
            }
        }
        PromptError::MaxTurnsError { .. } => Failure::TurnLimit,
        PromptError::UnknownToolCall { .. } => Failure::InvalidTool,
        PromptError::PromptCancelled { .. } => Failure::Cancelled,
        PromptError::MemoryError(_) => Failure::Internal,
    }
}

/// Map local errors to a fixed vocabulary; never emit arbitrary error text.
#[must_use]
pub fn io_failure(error: &std::io::Error) -> Failure {
    use std::io::ErrorKind;
    if let Some(failure) = error
        .get_ref()
        .and_then(|value| value.downcast_ref::<crate::operations::runtime::GitFailure>())
    {
        return failure.0;
    }
    match error.kind() {
        ErrorKind::NotFound => Failure::MissingFile,
        ErrorKind::PermissionDenied => Failure::PermissionDenied,
        ErrorKind::InvalidData | ErrorKind::InvalidInput => Failure::InvalidInput,
        _ => match error.to_string().as_str() {
            "changed paths outside bounds" => Failure::NoChanges,
            "task changes a protected path" => Failure::ProtectedPath,
            "task test edits are not authorized" => Failure::TestEdits,
            "source revision mismatch" | "verification changed protected source inputs" => {
                Failure::RuntimeMismatch
            }
            "invalid or oversized runtime input" | "oversized runtime input" => {
                Failure::InvalidInput
            }
            _ => Failure::Internal,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_agent::{client::AgentModelExt, completion::Prompt};
    use rig_core::{
        completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse},
        streaming::StreamingCompletionResponse,
    };

    struct RejectedModel;
    impl CompletionModel for RejectedModel {
        async fn completion(
            &self,
            _: CompletionRequest,
        ) -> Result<CompletionResponse, CompletionError> {
            Err(CompletionError::ProviderError(
                "private body and credential".into(),
            ))
        }
        async fn stream(
            &self,
            _: CompletionRequest,
        ) -> Result<StreamingCompletionResponse, CompletionError> {
            Err(CompletionError::ProviderError(
                "private streaming body".into(),
            ))
        }
    }

    #[tokio::test]
    async fn rig_failure_retains_stage_without_provider_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("activity.json");
        let log = ActivityLog::new(Some(path.clone()));
        let result = RejectedModel
            .into_agent_builder()
            .add_hook(log.clone())
            .build()
            .prompt("private prompt")
            .await;
        let error = result.err().ok_or("expected provider failure")?;
        log.fail(prompt_failure(&error));
        log.fail(Failure::Internal);
        let bytes = fs::read_to_string(path)?;
        assert!(!bytes.contains("private"));
        let events: Vec<Activity> = serde_json::from_str(&bytes)?;
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].stage, Stage::ModelRequest));
        assert!(matches!(events[1].failure, Some(Failure::ProviderRejected)));
        assert!(ayni_orchestrator_runtime::activity::valid(&events));
        Ok(())
    }

    #[test]
    fn tool_errors_are_classified_without_retaining_arguments_or_output()
    -> Result<(), Box<dyn std::error::Error>> {
        use rig_agent::tool::{ToolExecutionError, ToolOutput, ToolResult};
        let root = tempfile::tempdir()?;
        let path = root.path().join("activity.json");
        let log = ActivityLog::new(Some(path.clone()));
        log.record(Stage::ToolFinished);
        log.record_tool_failure(&ToolResult::failed(ToolExecutionError::invalid_args(
            "private arguments",
        )));
        log.record(Stage::ToolFinished);
        log.record_tool_failure(&ToolResult::success(ToolOutput::text("private output")));
        let bytes = fs::read_to_string(path)?;
        let events: Vec<Activity> = serde_json::from_str(&bytes)?;
        assert_eq!(events.len(), 3);
        assert!(matches!(events[1].failure, Some(Failure::InvalidInput)));
        assert!(!events[2].failed);
        assert!(!bytes.contains("private"));
        Ok(())
    }

    #[test]
    fn provider_status_is_classified_without_reading_its_body()
    -> Result<(), Box<dyn std::error::Error>> {
        use rig_core::ProviderResponseError;
        for (status, expected) in [
            ("401", "authentication"),
            ("429", "rate_limited"),
            ("503", "provider_unavailable"),
            ("400", "provider_rejected"),
        ] {
            let error = rig_agent::completion::PromptError::CompletionError(
                CompletionError::ProviderResponse(ProviderResponseError::new(
                    status.parse()?,
                    "private body",
                )),
            );
            assert_eq!(serde_json::to_value(prompt_failure(&error))?, expected);
        }
        Ok(())
    }

    #[test]
    fn bounded_log_keeps_latest_failure_and_order() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("activity.json");
        let log = ActivityLog::new(Some(path.clone()));
        for _ in 0..300 {
            log.record(Stage::ToolStarted);
        }
        log.fail(Failure::PermissionDenied);
        let events: Vec<Activity> = serde_json::from_slice(&fs::read(path)?)?;
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(events.last().map(|event| event.sequence), Some(301));
        assert!(events.last().is_some_and(|event| event.failed));
        assert!(ayni_orchestrator_runtime::activity::valid(&events));
        Ok(())
    }
}
