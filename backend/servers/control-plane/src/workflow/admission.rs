use super::{RequestContext, WorkflowStore, projections::receipt, v1, validation};
use ayni_orchestrator_observability::{
    TraceContext, current_context,
    workflow::{Workflow, finish},
};
use tonic::{Request, Response, Status};
use tracing::Instrument as _;
pub(super) async fn send(
    store: &WorkflowStore,
    request: Request<v1::SendMessageRequest>,
) -> Result<Response<v1::SendMessageResponse>, Status> {
    let context = RequestContext::new(&request);
    validation::send_message(request.get_ref()).map_err(|error| context.validation(&error))?;
    let incoming = request
        .metadata()
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .and_then(|parent| {
            TraceContext::parse(
                parent,
                request
                    .metadata()
                    .get("tracestate")
                    .and_then(|value| value.to_str().ok()),
            )
        });
    let request = request.into_inner();
    let span = Workflow {
        workspace_id: request.workspace_id.clone(),
        project_id: request.project_id.clone(),
        conversation_id: request.conversation_id.clone(),
        ..Workflow::default()
    }
    .span_with_context("api.message.admit", incoming.as_ref(), false);
    let started = std::time::Instant::now();
    let admitted = async {
        let diagnostic = current_context().or_else(|| incoming.clone());
        store
            .send_message_traced(
                &request.idempotency_key,
                &request.workspace_id,
                &request.project_id,
                &request.conversation_id,
                &request.content,
                diagnostic
                    .as_ref()
                    .map(|context| context.traceparent.as_str()),
            )
            .await
    }
    .instrument(span.clone())
    .await;
    ayni_orchestrator_workflow_postgres::metrics::observe(
        "workflow_message_admission_seconds",
        started.elapsed(),
    );
    ayni_orchestrator_workflow_postgres::metrics::increment(if admitted.is_ok() {
        "workflow_message_admissions_total"
    } else {
        "workflow_message_admission_failures_total"
    });
    finish(&span, admitted.is_ok());
    let accepted = admitted.map_err(|error| context.store(&error))?;
    span.record("turn_id", accepted.turn_id.as_str());
    span.record("event_id", accepted.command_id.as_str());
    let handoff = std::time::Instant::now();
    let response = context.response(v1::SendMessageResponse {
        receipt: Some(receipt(accepted)),
    });
    ayni_orchestrator_workflow_postgres::metrics::observe(
        "workflow_receipt_handoff_seconds",
        handoff.elapsed(),
    );
    Ok(response)
}
