//! Trusted operator consent and read-only retained assessment allowance.
use super::{RequestContext, WorkflowStore, plans, v1};
use tonic::{Request, Response, Status};

pub(super) async fn status(
    store: &WorkflowStore,
    request: Request<v1::GetOutcomeAssessmentStatusRequest>,
) -> Result<Response<v1::GetOutcomeAssessmentStatusResponse>, Status> {
    let context = RequestContext::new(&request);
    let value = request.into_inner();
    let status = store
        .outcome_assessment_status(plans::scope(
            &value.workspace_id,
            &value.project_id,
            &value.conversation_id,
        ))
        .await
        .map_err(|error| context.store(&error))?;
    Ok(context.response(v1::GetOutcomeAssessmentStatusResponse {
        authorization_id: status.authorization.id,
        configuration_revision_id: status.authorization.configuration_revision_id,
        max_assessments: status.authorization.max_assessments,
        admitted_assessments: status.admitted_assessments,
    }))
}

pub(super) async fn authorize(
    store: &WorkflowStore,
    request: Request<v1::AuthorizeOutcomeAssessmentsRequest>,
) -> Result<Response<v1::AuthorizeOutcomeAssessmentsResponse>, Status> {
    let context = RequestContext::new(&request);
    let value = request.into_inner();
    let authorization = store
        .authorize_outcome_assessments(
            plans::scope(
                &value.workspace_id,
                &value.project_id,
                &value.conversation_id,
            ),
            &value.idempotency_key,
            &value.expected_configuration_revision_id,
            value.max_assessments,
        )
        .await
        .map_err(|error| context.store(&error))?;
    Ok(context.response(v1::AuthorizeOutcomeAssessmentsResponse {
        authorization_id: authorization.id,
        configuration_revision_id: authorization.configuration_revision_id,
        max_assessments: authorization.max_assessments,
    }))
}
