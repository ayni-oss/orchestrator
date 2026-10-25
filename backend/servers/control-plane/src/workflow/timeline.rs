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
            })
            .collect(),
        snapshot_revision: page.snapshot_revision,
        next_before_revision: page.next_before_revision,
        retained_from_revision: page.retained_from_revision,
    }))
}
