use super::{WorkflowStore, errors::RequestContext, projections, v1};
use ayni_orchestrator_observability::workflow::{Workflow, finish};
use tonic::{Request, Response, Status};
use tracing::Instrument as _;

pub(super) async fn list(
    store: &WorkflowStore,
    request: Request<v1::ListExecutionTimelineRequest>,
) -> Result<Response<v1::ListExecutionTimelineResponse>, Status> {
    let context = RequestContext::new(&request);
    let value = request.into_inner();
    let span = Workflow {
        workspace_id: value.workspace_id.clone(),
        project_id: value.project_id.clone(),
        conversation_id: value.conversation_id.clone(),
        ..Workflow::default()
    }
    .span("execution.timeline.read");
    let result = store
        .list_execution_timeline(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
            value.snapshot_revision,
            value.before_revision,
            value.limit,
        )
        .instrument(span.clone())
        .await;
    finish(&span, result.is_ok());
    let page = result.map_err(|error| context.store(&error))?;
    Ok(context.response(v1::ListExecutionTimelineResponse {
        entries: page
            .entries
            .into_iter()
            .map(|entry| v1::ExecutionTimelineEntry {
                activity: Some(projections::activity(entry.activity)),
                kind: entry.kind,
                affected_resource_ids: entry.affected_resource_ids,
                trace_id: ayni_orchestrator_observability::TraceContext::parse(
                    &entry.traceparent,
                    None,
                )
                .map_or_else(String::new, |context| context.traceparent[3..35].to_owned()),
            })
            .collect(),
        snapshot_revision: page.snapshot_revision,
        next_before_revision: page.next_before_revision,
        retained_from_revision: page.retained_from_revision,
    }))
}

pub(super) async fn progress(
    store: &WorkflowStore,
    request: Request<v1::ListExecutionProgressRequest>,
) -> Result<Response<v1::ListExecutionProgressResponse>, Status> {
    let context = RequestContext::new(&request);
    let value = request.into_inner();
    let progress = store
        .list_execution_progress(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        )
        .await
        .map_err(|error| context.store(&error))?;
    Ok(context.response(v1::ListExecutionProgressResponse {
        progress: progress
            .into_iter()
            .map(|p| v1::LiveExecutionProgress {
                turn_id: p.turn_id,
                call_id: p.progress.call_id,
                session_id: p.session_id,
                generation: p.generation,
                sequence: p.progress.sequence,
                round: p.progress.round,
                stage: p.progress.stage,
                tool_name: p.progress.tool_name,
                tool_invocation_id: p.progress.tool_invocation_id,
                provisional_text: p.progress.provisional_text,
                observed_at: Some(projections::timestamp(p.observed_at)),
                stage_observed_at: Some(projections::timestamp(p.stage_observed_at)),
            })
            .collect(),
    }))
}
