use ayni_orchestrator_domain::{
    ModelInteractionOutcome, ModelUsageReport, ModelUsageStage, ProviderTokenUsage,
};
use rig_core::{
    completion::{
        CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
        ProviderCapabilities,
    },
    streaming::StreamingCompletionResponse,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct UsageTracker {
    inner: Arc<TrackerInner>,
}

struct TrackerInner {
    directory: PathBuf,
    stage: ModelUsageStage,
    provider: String,
    model: String,
    next_call: AtomicUsize,
    active: Mutex<HashMap<usize, ActiveCall>>,
}

#[derive(Clone)]
struct ActiveCall {
    started_at_ms: i64,
    started: Instant,
    tokens: ProviderTokenUsage,
}

impl UsageTracker {
    /// Creates a tracker after preparing a private durable spool directory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the spool cannot be created or made private.
    pub fn new(
        directory: impl Into<PathBuf>,
        stage: ModelUsageStage,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> io::Result<Self> {
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let next_call = next_call_number(&directory, stage);
        Ok(Self {
            inner: Arc::new(TrackerInner {
                directory,
                stage,
                provider: provider.into(),
                model: model.into(),
                next_call: AtomicUsize::new(next_call),
                active: Mutex::new(HashMap::new()),
            }),
        })
    }

    fn next_call(&self) -> usize {
        self.inner.next_call.fetch_add(1, Ordering::Relaxed)
    }

    fn begin(&self, turn: usize) -> io::Result<()> {
        let started_at_ms = unix_millis()?;
        let active = ActiveCall {
            started_at_ms,
            started: Instant::now(),
            tokens: ProviderTokenUsage::default(),
        };
        self.write_report(&ModelUsageReport {
            schema_version: ModelUsageReport::SCHEMA_VERSION,
            call_id: call_id(turn),
            stage: self.inner.stage,
            provider: self.inner.provider.clone(),
            model: self.inner.model.clone(),
            resolved_model: None,
            started_at_ms,
            completed_at_ms: None,
            latency_ms: None,
            outcome: ModelInteractionOutcome::InProgress,
            tokens: ProviderTokenUsage::default(),
        })?;
        self.lock_active()?.insert(turn, active);
        Ok(())
    }

    fn succeed(&self, turn: usize, response: &CompletionResponse) -> io::Result<()> {
        let completed_at_ms = unix_millis()?;
        let tokens = openai_usage(&response.raw);
        let active = {
            let mut calls = self.lock_active()?;
            calls.get_mut(&turn).map(|active| {
                active.tokens = tokens;
                (active.started_at_ms, active.started)
            })
        };
        let (started_at_ms, started) = active.unwrap_or_else(|| (completed_at_ms, Instant::now()));
        let report = ModelUsageReport {
            schema_version: ModelUsageReport::SCHEMA_VERSION,
            call_id: call_id(turn),
            stage: self.inner.stage,
            provider: self.inner.provider.clone(),
            model: self.inner.model.clone(),
            resolved_model: response
                .model
                .clone()
                .or_else(|| provider_model(&response.raw)),
            started_at_ms,
            completed_at_ms: Some(completed_at_ms),
            latency_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            outcome: ModelInteractionOutcome::Succeeded,
            tokens,
        };
        self.write_report(&report)?;
        self.lock_active()?.remove(&turn);
        Ok(())
    }

    fn fail(&self, turn: usize, provider_error: Option<&Value>) -> io::Result<()> {
        let Some(active) = self.lock_active()?.get(&turn).cloned() else {
            return Ok(());
        };
        self.write_failure(turn, &active, provider_error)?;
        self.lock_active()?.remove(&turn);
        Ok(())
    }

    fn write_failure(
        &self,
        turn: usize,
        active: &ActiveCall,
        provider_error: Option<&Value>,
    ) -> io::Result<()> {
        let completed_at_ms = unix_millis()?;
        let error_tokens = provider_error.map_or_else(ProviderTokenUsage::default, openai_usage);
        let tokens = if error_tokens.is_available() {
            error_tokens
        } else {
            active.tokens
        };
        self.write_report(&ModelUsageReport {
            schema_version: ModelUsageReport::SCHEMA_VERSION,
            call_id: call_id(turn),
            stage: self.inner.stage,
            provider: self.inner.provider.clone(),
            model: self.inner.model.clone(),
            resolved_model: provider_error.and_then(provider_model),
            started_at_ms: active.started_at_ms,
            completed_at_ms: Some(completed_at_ms),
            latency_ms: Some(
                u64::try_from(active.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ),
            outcome: ModelInteractionOutcome::Failed,
            tokens,
        })
    }

    /// Finalizes any outstanding provider request after the agent returns an error.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the bounded report cannot be written atomically.
    pub fn fail_pending(&self, provider_error: Option<&Value>) -> io::Result<()> {
        let pending = {
            let mut active = self.lock_active()?;
            active.drain().collect::<Vec<_>>()
        };
        for (turn, active) in pending {
            self.write_failure(turn, &active, provider_error)?;
        }
        Ok(())
    }

    fn lock_active(&self) -> io::Result<std::sync::MutexGuard<'_, HashMap<usize, ActiveCall>>> {
        self.inner
            .active
            .lock()
            .map_err(|_| io::Error::other("usage tracker lock poisoned"))
    }

    fn write_report(&self, report: &ModelUsageReport) -> io::Result<()> {
        fs::create_dir_all(&self.inner.directory)?;
        fs::set_permissions(&self.inner.directory, fs::Permissions::from_mode(0o700))?;
        let path =
            self.inner
                .directory
                .join(format!("{}-{}.json", report.stage.as_str(), report.call_id));
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(report).map_err(io::Error::other)?;
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temporary, path)?;
        File::open(&self.inner.directory)?.sync_all()
    }
}

/// Completion transport wrapper that records exactly one report per issued provider request.
pub struct TrackedModel<M> {
    inner: M,
    usage: UsageTracker,
}

impl<M> TrackedModel<M> {
    #[must_use]
    pub const fn new(inner: M, usage: UsageTracker) -> Self {
        Self { inner, usage }
    }
}

impl<M: CompletionModel> CompletionModel for TrackedModel<M> {
    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        let call = self.usage.next_call();
        self.usage.begin(call).map_err(|_| {
            CompletionError::ResponseError("provider usage spool unavailable".into())
        })?;
        match self.inner.completion(request).await {
            Ok(response) => {
                self.usage.succeed(call, &response).map_err(|_| {
                    CompletionError::ResponseError("provider usage spool unavailable".into())
                })?;
                Ok(response)
            }
            Err(error) => {
                let provider_error = error.provider_response_json().ok().flatten();
                self.usage
                    .fail(call, provider_error.as_ref())
                    .map_err(|_| {
                        CompletionError::ResponseError("provider usage spool unavailable".into())
                    })?;
                Err(error)
            }
        }
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        Err(CompletionError::ResponseError(
            "streaming is disabled for usage-tracked agents".into(),
        ))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
}

fn next_call_number(directory: &std::path::Path, stage: ModelUsageStage) -> usize {
    let prefix = format!("{}-call-", stage.as_str());
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| {
            name.strip_prefix(&prefix)?
                .strip_suffix(".json")?
                .parse::<usize>()
                .ok()
        })
        .max()
        .and_then(|maximum| maximum.checked_add(1))
        .unwrap_or(1)
}

fn call_id(turn: usize) -> String {
    format!("call-{turn}")
}

fn unix_millis() -> io::Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    i64::try_from(elapsed.as_millis()).map_err(io::Error::other)
}

fn provider_model(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty() && model.len() <= 128)
        .map(str::to_owned)
}

fn optional_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

/// Extracts only provider usage counters from `OpenAI` Responses or Chat Completions payloads.
/// The provider's raw payload is not retained.
#[must_use]
pub fn openai_usage(raw: &Value) -> ProviderTokenUsage {
    let usage = raw.get("usage").unwrap_or(raw);
    ProviderTokenUsage {
        input_tokens: optional_u64(
            usage
                .get("input_tokens")
                .or_else(|| usage.get("prompt_tokens")),
        ),
        output_tokens: optional_u64(
            usage
                .get("output_tokens")
                .or_else(|| usage.get("completion_tokens")),
        ),
        cached_input_tokens: optional_u64(
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .or_else(|| {
                    usage
                        .get("prompt_tokens_details")
                        .and_then(|details| details.get("cached_tokens"))
                }),
        ),
        reasoning_tokens: optional_u64(
            usage
                .get("output_tokens_details")
                .and_then(|details| details.get("reasoning_tokens"))
                .or_else(|| {
                    usage
                        .get("completion_tokens_details")
                        .and_then(|details| details.get("reasoning_tokens"))
                }),
        ),
        total_tokens: optional_u64(usage.get("total_tokens")),
    }
}

#[cfg(test)]
mod tests {
    use super::{TrackedModel, UsageTracker, openai_usage, provider_model};
    use ayni_orchestrator_domain::{ModelInteractionOutcome, ModelUsageReport, ModelUsageStage};
    use rig_core::{
        completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse},
        message::Message,
        streaming::StreamingCompletionResponse,
    };
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    struct FailingModel;

    impl CompletionModel for FailingModel {
        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, CompletionError> {
            Err(CompletionError::ProviderError("deliberate failure".into()))
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse, CompletionError> {
            Err(CompletionError::ProviderError("deliberate failure".into()))
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: vec![Message::user("redacted test prompt")],
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
            record_telemetry_content: false,
        }
    }

    #[test]
    fn extracts_complete_responses_usage() {
        let usage = openai_usage(&json!({"usage": {
            "input_tokens": 120, "output_tokens": 30, "total_tokens": 150,
            "input_tokens_details": {"cached_tokens": 80},
            "output_tokens_details": {"reasoning_tokens": 12}
        }}));
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(30));
        assert_eq!(usage.cached_input_tokens, Some(80));
        assert_eq!(usage.reasoning_tokens, Some(12));
        assert_eq!(usage.total_tokens, Some(150));
        assert_eq!(
            provider_model(&json!({"model": "gpt-5-2025-08-07"})).as_deref(),
            Some("gpt-5-2025-08-07")
        );
    }

    #[test]
    fn preserves_provider_specific_optional_fields_as_unknown() {
        let usage = openai_usage(&json!({"usage": {
            "prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14
        }}));
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(4));
        assert_eq!(usage.cached_input_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
        assert_eq!(openai_usage(&json!({})).input_tokens, None);
    }

    #[test]
    fn failed_report_retains_provider_usage_without_provider_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let tracker =
            UsageTracker::new(directory.path(), ModelUsageStage::Coding, "openai", "gpt-5")?;
        tracker.begin(1)?;
        tracker.fail(
            1,
            Some(&json!({
                "usage": {"input_tokens": 7, "output_tokens": 2, "total_tokens": 9},
                "completion": "sensitive provider content"
            })),
        )?;
        let bytes = std::fs::read(directory.path().join("coding-call-1.json"))?;
        assert_eq!(
            std::fs::metadata(directory.path())?.permissions().mode() & 0o777,
            0o700
        );
        let report: ModelUsageReport = serde_json::from_slice(&bytes)?;
        assert_eq!(report.outcome, ModelInteractionOutcome::Failed);
        assert_eq!(report.tokens.input_tokens, Some(7));
        assert_eq!(report.tokens.output_tokens, Some(2));
        assert!(!String::from_utf8(bytes)?.contains("sensitive provider content"));
        Ok(())
    }

    #[tokio::test]
    async fn failed_provider_request_is_spooled_without_sensitive_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let tracker =
            UsageTracker::new(directory.path(), ModelUsageStage::Coding, "openai", "gpt-5")?;
        let model = TrackedModel::new(FailingModel, tracker);
        assert!(model.completion(request()).await.is_err());

        let bytes = std::fs::read(directory.path().join("coding-call-1.json"))?;
        assert_eq!(
            std::fs::metadata(directory.path())?.permissions().mode() & 0o777,
            0o700
        );
        let report: ModelUsageReport = serde_json::from_slice(&bytes)?;
        assert_eq!(report.outcome, ModelInteractionOutcome::Failed);
        assert_eq!(report.tokens.input_tokens, None);
        let serialized = String::from_utf8(bytes)?;
        assert!(!serialized.contains("redacted test prompt"));
        assert!(!serialized.contains("deliberate failure"));

        let restarted = TrackedModel::new(
            FailingModel,
            UsageTracker::new(directory.path(), ModelUsageStage::Coding, "openai", "gpt-5")?,
        );
        assert!(restarted.completion(request()).await.is_err());
        assert!(directory.path().join("coding-call-2.json").is_file());
        Ok(())
    }
}
