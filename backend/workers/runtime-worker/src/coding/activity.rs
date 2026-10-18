//! Redacted worker interactions and errors, retained independently of recovery history.
use ayni_orchestrator_runtime::activity::{
    Activity, Failure, MAX_ACTIVITY_BYTES, MAX_DETAIL_BYTES, MAX_EVENTS, Stage,
};
use rig_agent::agent::{
    AgentHook, CompletionCallAction, CompletionCallEvent, CompletionResponseEvent, HookContext,
    ObservationAction, ToolCall, ToolCallAction, ToolResultAction, ToolResultEvent,
};
use std::io::Write;
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
    secrets: Vec<String>,
}

impl ActivityLog {
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        let events = path
            .as_ref()
            .and_then(|path| match fs::read(path) {
                Ok(bytes) if bytes.len() <= MAX_ACTIVITY_BYTES => {
                    serde_json::from_slice::<Vec<Activity>>(&bytes)
                        .ok()
                        .filter(|events| ayni_orchestrator_runtime::activity::valid(events))
                }
                Ok(_) => {
                    tracing::error!("retained activity exceeds its byte limit");
                    None
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    tracing::error!(%error, "retained activity could not be read");
                    None
                }
            })
            .unwrap_or_default();
        Self(Arc::new(Mutex::new(State {
            sequence: events.last().map_or(0, |event| event.sequence),
            stage: events.last().map(|event| event.stage),
            events,
            path,
            ..State::default()
        })))
    }

    pub fn record(&self, stage: Stage) {
        self.emit(Some(stage), None, "");
    }

    pub fn fail(&self, failure: Failure) {
        self.emit(None, Some(failure), "");
    }

    pub(super) fn failure_detail(&self, failure: Failure, details: &str) {
        self.emit(None, Some(failure), details);
    }

    pub fn error(&self, error: &std::io::Error) {
        self.emit(None, Some(io_failure(error)), &error.to_string());
    }

    pub fn detail(&self, stage: Stage, details: &str) {
        self.emit(Some(stage), None, details);
    }

    pub fn register_secret(&self, secret: &str) {
        if !secret.trim().is_empty()
            && let Ok(mut log) = self.0.lock()
        {
            log.secrets.push(secret.trim().to_owned());
        }
    }

    fn emit(&self, stage: Option<Stage>, failure: Option<Failure>, details: &str) {
        let Ok(mut log) = self.0.lock() else {
            return;
        };
        let failed = failure.is_some();
        let stage = stage.or(log.stage).unwrap_or(Stage::Request);
        log.stage = Some(stage);
        log.sequence = log.sequence.saturating_add(1);
        let details = super::diagnostics::redact(details, &log.secrets);
        let mut event = Activity {
            sequence: log.sequence,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|time| i64::try_from(time.as_millis()).ok())
                .unwrap_or(0),
            stage,
            failed,
            failure,
            details: details.clone(),
            truncated: false,
        };
        if failed {
            tracing::error!(stage = ?stage, failure = ?failure, sequence = event.sequence, details = %details, "worker stage failed");
        } else {
            tracing::info!(stage = ?stage, sequence = event.sequence, details = %details, message = stage.message());
        }
        if let Some(path) = &log.path {
            let result = serde_json::to_vec(&event)
                .map_err(std::io::Error::other)
                .and_then(|mut bytes| {
                    bytes.push(b'\n');
                    let mut file = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path.with_file_name("diagnostics.jsonl"))?;
                    file.write_all(&bytes)?;
                    file.sync_data()
                });
            if let Err(error) = result {
                tracing::error!(%error, "worker diagnostic log persistence failed");
            }
        }
        event.truncated = details.len() > MAX_DETAIL_BYTES;
        event.details = super::diagnostics::truncate(&details, MAX_DETAIL_BYTES);
        if log.events.len() == MAX_EVENTS {
            log.events.remove(0);
        }
        log.events.push(event);
        while serde_json::to_vec(&log.events).is_ok_and(|bytes| bytes.len() > MAX_ACTIVITY_BYTES)
            && log.events.len() > 1
        {
            log.events.remove(0);
        }
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
        event: CompletionResponseEvent<'_>,
    ) -> ObservationAction {
        self.record(Stage::ModelResponse);
        for (summary, text) in super::diagnostics::assistant_content(event.content) {
            self.detail(
                if summary {
                    Stage::ReasoningSummary
                } else {
                    Stage::AgentMessage
                },
                &text,
            );
        }
        ObservationAction::Continue
    }
    async fn on_tool_call(&self, _: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        self.detail(
            Stage::ToolStarted,
            &format!(
                "{} [{}]\n{}",
                event.tool_name, event.internal_call_id, event.args
            ),
        );
        ToolCallAction::Run
    }
    async fn on_tool_result(
        &self,
        _: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let output = event
            .presentation
            .as_text()
            .map_or_else(|| "Non-text tool output".to_owned(), str::to_owned);
        let details = format!(
            "{} [{}]\n{}",
            event.tool_name, event.internal_call_id, output
        );
        if let Some(error) = event
            .raw_result
            .error()
            .or_else(|| event.raw_result.refusal())
        {
            self.emit(
                Some(Stage::ToolFinished),
                Some(tool_failure(error)),
                &format!("{details}\n{}", error.message()),
            );
        } else if command_failed(event.tool_name, &output) {
            self.emit(
                Some(Stage::ToolFinished),
                Some(Failure::CommandExit),
                &details,
            );
        } else {
            self.detail(Stage::ToolFinished, &details);
        }
        ToolResultAction::Keep
    }
}

fn command_failed(tool: &str, output: &str) -> bool {
    tool == "run_command" && output.starts_with("exit=") && !output.starts_with("exit=Some(0)\n")
}

const fn tool_failure(error: &rig_agent::tool::ToolExecutionError) -> Failure {
    use rig_agent::tool::ToolErrorKind;
    match error.kind() {
        ToolErrorKind::InvalidArgs => Failure::InvalidInput,
        ToolErrorKind::Cancelled => Failure::Cancelled,
        ToolErrorKind::NotFound => Failure::MissingFile,
        ToolErrorKind::PermissionDenied => Failure::PermissionDenied,
        ToolErrorKind::RateLimited => Failure::RateLimited,
        ToolErrorKind::Network => Failure::Transport,
        _ => Failure::Internal,
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

/// Classify known failures while the diagnostic record retains redacted context.
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
        ErrorKind::TimedOut => Failure::Deadline,
        ErrorKind::NotFound => Failure::MissingFile,
        ErrorKind::PermissionDenied => Failure::PermissionDenied,
        ErrorKind::InvalidData | ErrorKind::InvalidInput => Failure::InvalidInput,
        _ => match error.to_string().as_str() {
            "protected policy changed" => Failure::PolicyChanged,
            "protected generated lock contract is invalid" => Failure::LockContract,
            "protected proposed lock lacks worker regeneration evidence" => {
                Failure::LockEvidenceMissing
            }
            "protected proposed lock regeneration evidence does not match final inputs" => {
                Failure::LockEvidenceMismatch
            }
            "task changes a protected dependency path outside reviewed environment scope" => {
                Failure::DependencyScope
            }
            "protected runtime inputs changed" => Failure::RuntimeMismatch,
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

pub(super) fn unsafe_change(error: &std::io::Error) -> bool {
    matches!(
        io_failure(error),
        Failure::ProtectedPath
            | Failure::TestEdits
            | Failure::PolicyChanged
            | Failure::DependencyScope
            | Failure::LockEvidenceMissing
            | Failure::LockEvidenceMismatch
    )
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
