use super::{
    BoxGrpcStream, GrpcApi, Request, Response, Status, api_validation, page_response, review_watch,
    v1, validate_id, validate_optional_id, with_store,
};
#[tonic::async_trait]
impl v1::review_service_server::ReviewService for GrpcApi {
    async fn list_change_requests(
        &self,
        request: Request<v1::ListChangeRequestsRequest>,
    ) -> Result<Response<v1::ListChangeRequestsResponse>, Status> {
        let context = self.context(&request, "ListChangeRequests")?;
        let value = request.get_ref();
        validate_optional_id(
            value.repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| validate_optional_id(value.candidate_id.as_deref(), "candidate_id"))
        .and_then(|()| {
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
            )
        })
        .and_then(|()| api_validation::page(value.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_change_requests(
                message.repository_connection_id.as_deref(),
                message.candidate_id.as_deref(),
                message.state,
                message.page.as_ref(),
            )
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListChangeRequestsResponse {
            change_requests: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_change_request(
        &self,
        request: Request<v1::GetChangeRequestRequest>,
    ) -> Result<Response<v1::GetChangeRequestResponse>, Status> {
        let context = self.context(&request, "GetChangeRequest")?;
        validate_id(&request.get_ref().change_request_id, "change_request_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.change_request(&message.change_request_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetChangeRequestResponse {
            change_request: Some(resource),
        }))
    }

    async fn publish_candidate(
        &self,
        request: Request<v1::PublishCandidateRequest>,
    ) -> Result<Response<v1::PublishCandidateResponse>, Status> {
        let context = self.context(&request, "PublishCandidate")?;
        api_validation::publish_candidate(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let command = super::publication_command(&key, &message);
        let resource = with_store(&self.state, move |store| {
            let id = ayni_orchestrator_application::publication::publish(store, command)
                .map_err(crate::adapters::sqlite::publication::publication_error)?;
            store.change_request(&id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::PublishCandidateResponse {
            change_request: Some(resource),
        }))
    }

    async fn list_assessments(
        &self,
        request: Request<v1::ListAssessmentsRequest>,
    ) -> Result<Response<v1::ListAssessmentsResponse>, Status> {
        let context = self.context(&request, "ListAssessments")?;
        let value = request.get_ref();
        validate_id(&value.change_request_id, "change_request_id")
            .and_then(|()| {
                value.head_revision.as_deref().map_or(Ok(()), |revision| {
                    api_validation::git_revision(revision, "head_revision")
                })
            })
            .and_then(|()| {
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
                )
            })
            .and_then(|()| api_validation::page(value.page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_assessments(
                &message.change_request_id,
                message.head_revision.as_deref(),
                message.conclusion,
                message.page.as_ref(),
            )
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListAssessmentsResponse {
            assessments: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_assessment(
        &self,
        request: Request<v1::GetAssessmentRequest>,
    ) -> Result<Response<v1::GetAssessmentResponse>, Status> {
        let context = self.context(&request, "GetAssessment")?;
        validate_id(&request.get_ref().assessment_id, "assessment_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.assessment(&message.assessment_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetAssessmentResponse {
            assessment: Some(resource),
        }))
    }

    async fn list_evidence_references(
        &self,
        request: Request<v1::ListEvidenceReferencesRequest>,
    ) -> Result<Response<v1::ListEvidenceReferencesResponse>, Status> {
        let context = self.context(&request, "ListEvidenceReferences")?;
        validate_id(&request.get_ref().assessment_id, "assessment_id")
            .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_evidence_references(&message.assessment_id, message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListEvidenceReferencesResponse {
            evidence_references: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_evidence_reference(
        &self,
        request: Request<v1::GetEvidenceReferenceRequest>,
    ) -> Result<Response<v1::GetEvidenceReferenceResponse>, Status> {
        let context = self.context(&request, "GetEvidenceReference")?;
        validate_id(
            &request.get_ref().evidence_reference_id,
            "evidence_reference_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.evidence_reference(&message.evidence_reference_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetEvidenceReferenceResponse {
            evidence_reference: Some(resource),
        }))
    }

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
        let stream = review_watch(
            self.state.clone(),
            self.shutdown.clone(),
            request.into_inner(),
            context.request_id.clone(),
        );
        Ok(context.response(Box::pin(stream)))
    }
}
