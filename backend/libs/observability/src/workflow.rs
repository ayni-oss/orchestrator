//! Credential-free workflow diagnostics. Identifiers correlate spans across restarts;
//! they never confer execution authority or replace durable receipts.
use crate::TraceContext;
use opentelemetry::trace::TraceContextExt as _;
use tracing::{Span, field};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub const WORKFLOW_TARGET: &str = "ayni_orchestrator_observability::workflow";

#[derive(Clone, Debug, Default)]
pub struct Workflow {
    pub workspace_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub provider_call_id: String,
    pub tool_invocation_id: String,
    pub parent_attempt_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub run_id: String,
    pub plan_id: String,
    pub plan_version: u64,
    pub proposal_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub session_id: String,
    pub generation: u64,
    pub model: String,
    pub profile_revision: String,
}

impl Workflow {
    /// Only bounded identifiers and numeric measurements enter exported diagnostics.
    #[must_use]
    pub fn span(&self, stage: &'static str) -> Span {
        let safe = identifier;
        tracing::info_span!(target: WORKFLOW_TARGET, "workflow",
            stage, workspace_id = safe(&self.workspace_id),
            turn_id = safe(&self.turn_id), event_id = safe(&self.event_id),
            provider_call_id = safe(&self.provider_call_id), tool_invocation_id = safe(&self.tool_invocation_id),
            parent_attempt_id = safe(&self.parent_attempt_id), replay = field::Empty,
            project_id = safe(&self.project_id),
            conversation_id = safe(&self.conversation_id), message_id = safe(&self.message_id),
            run_id = safe(&self.run_id), plan_id = safe(&self.plan_id),
            plan_version = self.plan_version, proposal_id = safe(&self.proposal_id),
            task_id = safe(&self.task_id), attempt_id = safe(&self.attempt_id),
            session_id = safe(&self.session_id), generation = self.generation,
            model = safe(&self.model), profile_revision = safe(&self.profile_revision),
            outcome = field::Empty, tool = field::Empty, model_calls = field::Empty,
            input_tokens = field::Empty, output_tokens = field::Empty,
            reason_code = field::Empty, execution = field::Empty,
            quality = field::Empty, publication = field::Empty)
    }

    /// Continue an accepted operation across a process boundary. Replay links to
    /// the originating context and records a separate bounded operation span.
    #[must_use]
    pub fn span_with_context(
        &self,
        stage: &'static str,
        context: Option<&TraceContext>,
        replay: bool,
    ) -> Span {
        let span = self.span(stage);
        span.record("replay", replay);
        if let Some(context) = context {
            let parent = context.context();
            let _ = span.set_parent(parent.clone());
            if replay {
                span.add_link(parent.span().span_context().clone());
            }
        }
        span
    }

    /// Read only allowlisted identities from an already bounded command/report.
    /// Content, raw errors and tool arguments never become span attributes.
    #[must_use]
    pub fn from_event(bytes: &[u8]) -> Self {
        let Ok(event) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            return Self::default();
        };
        let binding = &event["data"]["binding"];
        let value = |snake: &str, camel: &str, extension: &str| {
            binding
                .get(snake)
                .or_else(|| binding.get(camel))
                .or_else(|| event.get(extension))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let session = &event["data"]["session"];
        Self {
            workspace_id: value("workspace_id", "workspaceId", "workspaceid"),
            project_id: value("project_id", "projectId", "projectid"),
            conversation_id: value("conversation_id", "conversationId", "conversationid"),
            turn_id: value("turn_id", "turnId", "turnid"),
            event_id: event["id"].as_str().unwrap_or_default().to_owned(),
            session_id: session
                .get("session_id")
                .or_else(|| session.get("sessionId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            generation: binding
                .get("executor_generation")
                .or_else(|| binding.get("executorGeneration"))
                .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
                .unwrap_or_default(),
            ..Self::default()
        }
    }

    /// An observation may precede a transaction commit. It is diagnostic evidence only.
    pub fn observe(&self, stage: &'static str, outcome: &str) {
        let span = self.span(stage);
        span.record("outcome", super::bounded_correlation(outcome));
    }
}

/// Task attempts and retained recovery identities give independent operation
/// traces a stable correlation even when no process survives between stages.
///
/// This context is diagnostic only and grants no authority.
#[must_use]
pub fn identity_context(identity: &str) -> Option<TraceContext> {
    let compact: String = identity
        .chars()
        .filter(|character| *character != '-')
        .collect();
    if compact.len() != 32 || !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    TraceContext::parse(&format!("00-{compact}-{}-01", &compact[..16]), None)
}

/// Add causality without classifying a normal operation as replay.
pub fn link_context(span: &Span, context: Option<&TraceContext>) {
    if let Some(context) = context {
        span.add_link(context.context().span().span_context().clone());
    }
}

/// Mark a proven retained-result replay after its ledger lookup completes.
pub fn record_replay(span: &Span, context: Option<&TraceContext>) {
    span.record("replay", true);
    link_context(span, context);
}

/// Link a bounded continuation to its explicitly recorded parent attempt.
pub fn link_recovery(span: &Span, parent_attempt_id: &str) {
    if let Some(context) = identity_context(parent_attempt_id) {
        span.add_link(context.context().span().span_context().clone());
    }
}

/// Project model-requested names only after exact configured-tool membership.
/// Invalid configured names and unknown requests share a fixed diagnostic label.
#[must_use]
pub fn configured_tool_name<'a>(
    requested: &'a str,
    configured: impl IntoIterator<Item = &'a str>,
) -> &'a str {
    let bounded = !requested.is_empty()
        && requested.len() <= 128
        && requested.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':' | b'@')
        });
    if bounded && configured.into_iter().any(|name| name == requested) {
        requested
    } else {
        "unknown_tool"
    }
}

fn identifier(value: &str) -> &str {
    if value.is_empty() {
        ""
    } else {
        super::bounded_correlation(value)
    }
}

/// Classify failures without accepting arbitrary upstream messages or payloads.
pub fn finish(span: &Span, succeeded: bool) {
    span.record("outcome", if succeeded { "succeeded" } else { "failed" });
    if !succeeded {
        super::record_failure(span, "workflow_operation_failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::{Layer, prelude::*};

    #[test]
    fn exported_tool_names_require_safe_configured_membership()
    -> Result<(), Box<dyn std::error::Error>> {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer().with_tracer(provider.tracer("tool-name-projection")),
        );
        let malicious = "read_repository_file\nprivate-content";
        let unconfigured = "unconfigured-sensitive-name";
        let configured = ["read_repository_file", malicious];
        tracing::subscriber::with_default(subscriber, || {
            for requested in [malicious, unconfigured, "read_repository_file", ""] {
                let projected = configured_tool_name(requested, configured);
                let span = Workflow::default().span("provider.tool.execute");
                span.record("tool", projected);
                finish(&span, true);
            }
        });
        provider.force_flush()?;
        let spans = exporter.get_finished_spans()?;
        assert_eq!(spans.len(), 4);
        let diagnostics = format!("{spans:?}");
        assert!(!diagnostics.contains("private-content"));
        assert!(!diagnostics.contains(unconfigured));
        assert!(diagnostics.contains("unknown_tool"));
        assert!(diagnostics.contains("read_repository_file"));
        provider.shutdown()?;
        Ok(())
    }

    #[test]
    fn command_context_crosses_operation_boundaries_and_replay_links_without_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("workflow-propagation-test")),
        );
        let parent = crate::TraceContext::parse(
            "00-7431fc7927bb4f208986f2a4f2418dba-3f49c016c2df402b-01",
            None,
        )
        .ok_or("context missing")?;
        tracing::subscriber::with_default(subscriber, || {
            let admitted = Workflow {
                workspace_id: "workspace-1".into(),
                conversation_id: "conversation-1".into(),
                turn_id: "turn-1".into(),
                ..Workflow::default()
            }
            .span_with_context("api.message.admit", Some(&parent), false);
            let bytes = admitted.in_scope(|| crate::with_event_context(serde_json::to_vec(&serde_json::json!({
                "id":"event-1", "data":{"binding":{"workspaceId":"workspace-1","conversationId":"conversation-1","turnId":"turn-1","executorGeneration":"2"},"inputContent":"private prompt","api_key":"private key"}
            })).unwrap_or_default(), 16384));
            let retained =
                crate::event_context(&bytes).unwrap_or_else(|| panic!("frozen event context"));
            let workflow = Workflow::from_event(&bytes);
            assert_eq!(workflow.turn_id, "turn-1");
            assert_eq!(workflow.generation, 2);
            let publish = workflow.span_with_context("outbox.publish", Some(&retained), false);
            finish(&publish, true);
            let replay =
                workflow.span_with_context("supervisor.command.receive", Some(&retained), true);
            finish(&replay, true);
            finish(&admitted, true);
        });
        provider.force_flush()?;
        let spans = exporter.get_finished_spans()?;
        assert_eq!(spans.len(), 3);
        assert!(
            spans
                .iter()
                .all(|span| span.span_context.trace_id().to_string()
                    == "7431fc7927bb4f208986f2a4f2418dba")
        );
        assert!(spans.iter().any(|span| !span.links.links.is_empty()));
        let diagnostics = format!("{spans:?}");
        assert!(!diagnostics.contains("private prompt"));
        assert!(!diagnostics.contains("private key"));
        provider.shutdown()?;
        Ok(())
    }

    #[test]
    fn workflow_export_correlates_safe_identities_and_classifies_failures_without_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("workflow-test"))
                .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                    m.target() == WORKFLOW_TARGET
                })),
        );
        tracing::subscriber::with_default(subscriber, || {
            let workflow = Workflow {
                project_id: "project-1".into(),
                conversation_id: "conversation-1".into(),
                message_id: "message-1".into(),
                run_id: "run-1".into(),
                profile_revision: "sha256:123".into(),
                model: "invalid\nprivate-content".into(),
                ..Workflow::default()
            };
            let parent = workflow.span("planner.request");
            parent.in_scope(|| {
                tracing::info!(target: "ayni_orchestrator_worker", prompt="private prompt", api_key="private key");
                let child = workflow.span("agent.tool");
                child.record("tool", "read_repository_file");
                finish(&child, false);
            });
            finish(&parent, true);
        });
        provider.force_flush()?;
        let spans = exporter.get_finished_spans()?;
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].parent_span_id, spans[1].span_context.span_id());
        let output = format!("{spans:?}");
        for excluded in ["private prompt", "private key", "private-content"] {
            assert!(!output.contains(excluded));
        }
        for included in [
            "project-1",
            "conversation-1",
            "message-1",
            "run-1",
            "sha256:123",
            "workflow_operation_failed",
        ] {
            assert!(output.contains(included));
        }
        provider.shutdown()?;
        Ok(())
    }
}
