use super::*;

#[tonic::async_trait]
impl v1::provider_service_server::ProviderService for GrpcApi {
    async fn list_provider_models(
        &self,
        request: Request<v1::ListProviderModelsRequest>,
    ) -> Result<Response<v1::ListProviderModelsResponse>, Status> {
        let context = self.context(&request, "ListProviderModels")?;
        api_validation::page(request.get_ref().page.as_ref())
            .map_err(|error| context.validation(&error))?;
        let catalog = self.state.model_catalog().ok_or_else(|| {
            context.status(
                Code::Unavailable,
                v1::ErrorReason::ProviderUnavailable,
                "the provider catalog is unavailable",
                None,
                None,
                None,
            )
        })?;
        let available = catalog.available().await.map_err(|_| {
            context.status(
                Code::Unavailable,
                v1::ErrorReason::ProviderUnavailable,
                "the provider catalog is unavailable",
                None,
                None,
                None,
            )
        })?;
        let size = request.get_ref().page.as_ref().map_or(50_usize, |page| {
            if page.page_size == 0 {
                50
            } else {
                usize::try_from(page.page_size).unwrap_or(100).min(100)
            }
        });
        let models = available
            .models
            .iter()
            .take(size)
            .map(|value| v1::ProviderModel {
                id: value.id.clone(),
                name: value.name.clone(),
                owned_by: value.owned_by.clone(),
                created_at: value
                    .created_at
                    .and_then(|seconds| i64::try_from(seconds).ok())
                    .map(|seconds| Timestamp { seconds, nanos: 0 }),
                context_length: value.context_length,
                max_output_tokens: value.max_output_tokens,
            })
            .collect();
        Ok(context.response(v1::ListProviderModelsResponse {
            provider: available.provider.to_owned(),
            models,
            page: Some(v1::PageResponse {
                next_page_token: String::new(),
                snapshot_cursor: 0,
            }),
            observed_at: i64::try_from(available.observed_at)
                .ok()
                .map(|seconds| Timestamp { seconds, nanos: 0 }),
        }))
    }
}

#[tonic::async_trait]
impl v1::development_task_service_server::DevelopmentTaskService for GrpcApi {
    async fn list_development_tasks(
        &self,
        request: Request<v1::ListDevelopmentTasksRequest>,
    ) -> Result<Response<v1::ListDevelopmentTasksResponse>, Status> {
        let context = self.context(&request, "ListDevelopmentTasks")?;
        let value = request.get_ref();
        validate_optional_id(value.project_id.as_deref(), "project_id")
            .map_err(|e| context.validation(&e))?;
        validate_optional_id(
            value.repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            validate_optional_id(value.runtime_profile_id.as_deref(), "runtime_profile_id")
        })
        .and_then(|()| {
            api_validation::enum_filter(
                value.state,
                "state",
                &[
                    v1::DevelopmentTaskState::Queued as i32,
                    v1::DevelopmentTaskState::WaitingForRuntime as i32,
                    v1::DevelopmentTaskState::Running as i32,
                    v1::DevelopmentTaskState::Succeeded as i32,
                    v1::DevelopmentTaskState::Failed as i32,
                    v1::DevelopmentTaskState::Cancelled as i32,
                    v1::DevelopmentTaskState::DeadlineExceeded as i32,
                    v1::DevelopmentTaskState::Unsupported as i32,
                ],
            )
        })
        .and_then(|()| api_validation::page(value.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_development_tasks(
                message.project_id.as_deref(),
                message.repository_connection_id.as_deref(),
                message.runtime_profile_id.as_deref(),
                message.state,
                message.page.as_ref(),
            )
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListDevelopmentTasksResponse {
            development_tasks: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_development_task(
        &self,
        request: Request<v1::GetDevelopmentTaskRequest>,
    ) -> Result<Response<v1::GetDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "GetDevelopmentTask")?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.development_task(&message.development_task_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetDevelopmentTaskResponse {
            development_task: Some(resource),
        }))
    }

    async fn create_development_task(
        &self,
        request: Request<v1::CreateDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CreateDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CreateDevelopmentTask")?;
        self.context(&request, "PublishCandidate")?;
        api_validation::create_development_task(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let command = task_mapping::task_command(&key, &message)
            .map_err(|error| context.store_error(&error))?;
        let (task, attempt) = with_store(&self.state, move |store| {
            task_mapping::admit_task(store, command)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CreateDevelopmentTaskResponse {
            development_task: Some(task),
            initial_attempt: Some(attempt),
        }))
    }

    async fn cancel_development_task(
        &self,
        request: Request<v1::CancelDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CancelDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CancelDevelopmentTask")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let command = task_mapping::cancel_command(&key, &message);
        let task = with_store(&self.state, move |store| {
            task_mapping::cancel_task(store, command)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CancelDevelopmentTaskResponse {
            development_task: Some(task),
        }))
    }

    async fn retry_development_task(
        &self,
        request: Request<v1::RetryDevelopmentTaskRequest>,
    ) -> Result<Response<v1::RetryDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "RetryDevelopmentTask")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let attempt = with_store(&self.state, move |store| {
            store.retry_development_task(&key, &message)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::RetryDevelopmentTaskResponse {
            attempt: Some(attempt),
        }))
    }

    async fn list_development_attempts(
        &self,
        request: Request<v1::ListDevelopmentAttemptsRequest>,
    ) -> Result<Response<v1::ListDevelopmentAttemptsResponse>, Status> {
        let context = self.context(&request, "ListDevelopmentAttempts")?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_development_attempts(&message.development_task_id, message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListDevelopmentAttemptsResponse {
            attempts: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_development_attempt(
        &self,
        request: Request<v1::GetDevelopmentAttemptRequest>,
    ) -> Result<Response<v1::GetDevelopmentAttemptResponse>, Status> {
        let context = self.context(&request, "GetDevelopmentAttempt")?;
        validate_id(
            &request.get_ref().development_attempt_id,
            "development_attempt_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.development_attempt(&message.development_attempt_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetDevelopmentAttemptResponse {
            attempt: Some(resource),
        }))
    }

    async fn list_candidates(
        &self,
        request: Request<v1::ListCandidatesRequest>,
    ) -> Result<Response<v1::ListCandidatesResponse>, Status> {
        let context = self.context(&request, "ListCandidates")?;
        let value = request.get_ref();
        validate_optional_id(value.development_task_id.as_deref(), "development_task_id")
            .and_then(|()| {
                validate_optional_id(
                    value.development_attempt_id.as_deref(),
                    "development_attempt_id",
                )
            })
            .and_then(|()| api_validation::page(value.page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_candidates(
                message.development_task_id.as_deref(),
                message.development_attempt_id.as_deref(),
                message.page.as_ref(),
            )
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListCandidatesResponse {
            candidates: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_candidate(
        &self,
        request: Request<v1::GetCandidateRequest>,
    ) -> Result<Response<v1::GetCandidateResponse>, Status> {
        let context = self.context(&request, "GetCandidate")?;
        validate_id(&request.get_ref().candidate_id, "candidate_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.candidate(&message.candidate_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetCandidateResponse {
            candidate: Some(resource),
        }))
    }

    type WatchDevelopmentTaskStream = BoxGrpcStream<v1::WatchDevelopmentTaskResponse>;

    async fn watch_development_task(
        &self,
        request: Request<v1::WatchDevelopmentTaskRequest>,
    ) -> Result<Response<Self::WatchDevelopmentTaskStream>, Status> {
        let context = self.context(&request, "WatchDevelopmentTask")?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let task_id = message.development_task_id.clone();
        with_store(&self.state, move |store| {
            store.development_task(&task_id).map(|_| ())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        let stream = development_watch(
            self.state.clone(),
            self.shutdown.clone(),
            message,
            context.request_id.clone(),
        );
        Ok(context.response(Box::pin(stream)))
    }
}
