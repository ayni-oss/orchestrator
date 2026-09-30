#[tonic::async_trait]
impl v1::review_service_server::ReviewService for GrpcApi {
    stub_unary!(
        list_change_requests,
        v1::ListChangeRequestsRequest,
        v1::ListChangeRequestsResponse,
        "ListChangeRequests",
        |value: &v1::ListChangeRequestsRequest| {
            validate_optional_id(
                value.repository_connection_id.as_deref(),
                "repository_connection_id",
            )?;
            validate_optional_id(value.candidate_id.as_deref(), "candidate_id")?;
            api_validation::enum_filter(
                value.state,
                "state",
                &[
                    v1::ChangeRequestState::Publishing as i32,
                    v1::ChangeRequestState::Draft as i32,
                    v1::ChangeRequestState::Open as i32,
                    v1::ChangeRequestState::Closed as i32,
                    v1::ChangeRequestState::Merged as i32,
                    v1::ChangeRequestState::Failed as i32,
                ],
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_change_request,
        v1::GetChangeRequestRequest,
        v1::GetChangeRequestResponse,
        "GetChangeRequest",
        |value: &v1::GetChangeRequestRequest| validate_id(
            &value.change_request_id,
            "change_request_id"
        )
    );

    async fn publish_candidate(
        &self,
        request: Request<v1::PublishCandidateRequest>,
    ) -> Result<Response<v1::PublishCandidateResponse>, Status> {
        let context = self.context(&request, "PublishCandidate")?;
        let message = request.into_inner();
        api_validation::publish_candidate(&message).map_err(|error| context.validation(&error))?;
        let _ = context.idempotency_key(&message.idempotency_key)?;
        Err(context.unimplemented())
    }

    stub_unary!(
        list_assessments,
        v1::ListAssessmentsRequest,
        v1::ListAssessmentsResponse,
        "ListAssessments",
        |value: &v1::ListAssessmentsRequest| {
            validate_id(&value.change_request_id, "change_request_id")?;
            if let Some(revision) = value.head_revision.as_deref() {
                api_validation::git_revision(revision, "head_revision")?;
            }
            api_validation::enum_filter(
                value.conclusion,
                "conclusion",
                &[
                    v1::AssessmentConclusion::Succeeded as i32,
                    v1::AssessmentConclusion::QualityFailed as i32,
                    v1::AssessmentConclusion::Incomplete as i32,
                    v1::AssessmentConclusion::PolicyRejected as i32,
                    v1::AssessmentConclusion::Stale as i32,
                    v1::AssessmentConclusion::Cancelled as i32,
                    v1::AssessmentConclusion::InfrastructureFailed as i32,
                    v1::AssessmentConclusion::Unsupported as i32,
                ],
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_assessment,
        v1::GetAssessmentRequest,
        v1::GetAssessmentResponse,
        "GetAssessment",
        |value: &v1::GetAssessmentRequest| validate_id(&value.assessment_id, "assessment_id")
    );
    stub_unary!(
        list_evidence_references,
        v1::ListEvidenceReferencesRequest,
        v1::ListEvidenceReferencesResponse,
        "ListEvidenceReferences",
        |value: &v1::ListEvidenceReferencesRequest| {
            validate_id(&value.assessment_id, "assessment_id")?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_evidence_reference,
        v1::GetEvidenceReferenceRequest,
        v1::GetEvidenceReferenceResponse,
        "GetEvidenceReference",
        |value: &v1::GetEvidenceReferenceRequest| validate_id(
            &value.evidence_reference_id,
            "evidence_reference_id"
        )
    );

    type WatchReviewsStream = BoxGrpcStream<v1::WatchReviewsResponse>;

    async fn watch_reviews(
        &self,
        request: Request<v1::WatchReviewsRequest>,
    ) -> Result<Response<Self::WatchReviewsStream>, Status> {
        let context = self.context(&request, "WatchReviews")?;
        validate_optional_id(
            request.get_ref().repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            validate_optional_id(
                request.get_ref().change_request_id.as_deref(),
                "change_request_id",
            )
        })
        .map_err(|error| context.validation(&error))?;
        Err(context.unimplemented())
    }
}
