//! Credential-free workflow diagnostics. Identifiers correlate spans across restarts;
//! they never confer execution authority or replace durable receipts.
use tracing::{Span, field};

pub const WORKFLOW_TARGET: &str = "ayni_orchestrator_observability::workflow";

#[derive(Clone, Debug, Default)]
pub struct Workflow {
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
            stage, project_id = safe(&self.project_id),
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

    /// An observation may precede a transaction commit. It is diagnostic evidence only.
    pub fn observe(&self, stage: &'static str, outcome: &str) {
        let span = self.span(stage);
        span.record("outcome", super::bounded_correlation(outcome));
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
