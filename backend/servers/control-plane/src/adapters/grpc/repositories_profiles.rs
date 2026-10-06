use super::onboarding_error::{provider_error, status as onboarding_error};
use super::{
    BoxGrpcStream, GrpcApi, Request, Response, Status, api_validation, page_response,
    profile_watch, repository_watch, v1, validate_id, validate_optional_id, with_store,
};

#[tonic::async_trait]
impl v1::repository_service_server::RepositoryService for GrpcApi {
    async fn list_github_installations(
        &self,
        request: Request<v1::ListGithubInstallationsRequest>,
    ) -> Result<Response<v1::ListGithubInstallationsResponse>, Status> {
        let context = self.context(&request, "ListGithubInstallations")?;
        let value = self
            .state
            .onboarding_source()
            .map_err(onboarding_error)?
            .github_installations()
            .await
            .map_err(provider_error)?;
        let installations = value["installations"]
            .as_array()
            .ok_or_else(|| provider_error("github_installations_invalid"))?
            .iter()
            .map(|item| {
                Ok(v1::GithubInstallationOption {
                    installation_id: item["installation_id"]
                        .as_u64()
                        .ok_or_else(|| provider_error("github_installation_invalid"))?
                        .to_string(),
                    owner: item["owner"]
                        .as_str()
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| provider_error("github_owner_invalid"))?
                        .to_owned(),
                    app_name: value["app_name"]
                        .as_str()
                        .unwrap_or("GitHub App")
                        .to_owned(),
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;
        Ok(context.response(v1::ListGithubInstallationsResponse { installations }))
    }

    async fn list_github_installation_repositories(
        &self,
        request: Request<v1::ListGithubInstallationRepositoriesRequest>,
    ) -> Result<Response<v1::ListGithubInstallationRepositoriesResponse>, Status> {
        let context = self.context(&request, "ListGithubInstallationRepositories")?;
        let id = request
            .get_ref()
            .installation_id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| Status::invalid_argument("provider_installation_invalid"))?;
        let value = self
            .state
            .onboarding_source()
            .map_err(onboarding_error)?
            .installation_repositories(id)
            .await
            .map_err(provider_error)?;
        let repositories = value["repositories"]
            .as_array()
            .ok_or_else(|| provider_error("github_repositories_invalid"))?
            .iter()
            .map(|item| {
                Ok(v1::GithubRepositoryOption {
                    repository_id: item["id"]
                        .as_u64()
                        .ok_or_else(|| provider_error("github_repository_invalid"))?
                        .to_string(),
                    owner: item["owner"]["login"]
                        .as_str()
                        .ok_or_else(|| provider_error("github_repository_invalid"))?
                        .to_owned(),
                    name: item["name"]
                        .as_str()
                        .ok_or_else(|| provider_error("github_repository_invalid"))?
                        .to_owned(),
                    default_branch: item["default_branch"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;
        Ok(context.response(v1::ListGithubInstallationRepositoriesResponse { repositories }))
    }

    async fn connect_github_repository(
        &self,
        request: Request<v1::ConnectGithubRepositoryRequest>,
    ) -> Result<Response<v1::ConnectGithubRepositoryResponse>, Status> {
        let context = self.context(&request, "ConnectGithubRepository")?;
        context.idempotency_key(&request.get_ref().idempotency_key)?;
        let repository = self
            .state
            .connect_github(request.get_ref())
            .await
            .map_err(onboarding_error)?;
        Ok(context.response(v1::ConnectGithubRepositoryResponse {
            repository_connection: Some(repository),
        }))
    }

    async fn list_repository_connections(
        &self,
        request: Request<v1::ListRepositoryConnectionsRequest>,
    ) -> Result<Response<v1::ListRepositoryConnectionsResponse>, Status> {
        let context = self.context(&request, "ListRepositoryConnections")?;
        let message = request.into_inner();
        api_validation::enum_filter(
            message.status,
            "status",
            &[
                v1::RepositoryConnectionStatus::Connecting as i32,
                v1::RepositoryConnectionStatus::Connected as i32,
                v1::RepositoryConnectionStatus::Degraded as i32,
                v1::RepositoryConnectionStatus::Revoked as i32,
                v1::RepositoryConnectionStatus::Failed as i32,
            ],
        )
        .and_then(|()| api_validation::page(message.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let page = with_store(&self.state, move |store| {
            store.list_repository_connections(message.status, message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRepositoryConnectionsResponse {
            repository_connections: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_repository_connection(
        &self,
        request: Request<v1::GetRepositoryConnectionRequest>,
    ) -> Result<Response<v1::GetRepositoryConnectionResponse>, Status> {
        let context = self.context(&request, "GetRepositoryConnection")?;
        api_validation::resource_id(
            &request.get_ref().repository_connection_id,
            "repository_connection_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.repository_connection(&message.repository_connection_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRepositoryConnectionResponse {
            repository_connection: Some(resource),
        }))
    }

    async fn create_repository_connection(
        &self,
        request: Request<v1::CreateRepositoryConnectionRequest>,
    ) -> Result<Response<v1::CreateRepositoryConnectionResponse>, Status> {
        let context = self.context(&request, "CreateRepositoryConnection")?;
        api_validation::create_repository_connection(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.create_repository_connection(&key, &message)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CreateRepositoryConnectionResponse {
            repository_connection: Some(resource),
        }))
    }

    async fn list_repository_snapshots(
        &self,
        request: Request<v1::ListRepositorySnapshotsRequest>,
    ) -> Result<Response<v1::ListRepositorySnapshotsResponse>, Status> {
        let context = self.context(&request, "ListRepositorySnapshots")?;
        validate_id(
            &request.get_ref().repository_connection_id,
            "repository_connection_id",
        )
        .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store
                .list_repository_snapshots(&message.repository_connection_id, message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRepositorySnapshotsResponse {
            repository_snapshots: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_repository_snapshot(
        &self,
        request: Request<v1::GetRepositorySnapshotRequest>,
    ) -> Result<Response<v1::GetRepositorySnapshotResponse>, Status> {
        let context = self.context(&request, "GetRepositorySnapshot")?;
        validate_id(
            &request.get_ref().repository_snapshot_id,
            "repository_snapshot_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.repository_snapshot(&message.repository_snapshot_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRepositorySnapshotResponse {
            repository_snapshot: Some(resource),
        }))
    }

    type WatchRepositoriesStream = BoxGrpcStream<v1::WatchRepositoriesResponse>;

    async fn watch_repositories(
        &self,
        request: Request<v1::WatchRepositoriesRequest>,
    ) -> Result<Response<Self::WatchRepositoriesStream>, Status> {
        let context = self.context(&request, "WatchRepositories")?;
        validate_optional_id(
            request.get_ref().repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let state = self.state.clone();
        let shutdown = self.shutdown.clone();
        let request_id = context.request_id.clone();
        let stream = repository_watch(state, shutdown, message, request_id);
        Ok(context.response(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl v1::runtime_profile_service_server::RuntimeProfileService for GrpcApi {
    async fn inspect_ayni_repository(
        &self,
        request: Request<v1::InspectAyniRepositoryRequest>,
    ) -> Result<Response<v1::InspectAyniRepositoryResponse>, Status> {
        let context = self.context(&request, "InspectAyniRepository")?;
        validate_id(
            &request.get_ref().repository_connection_id,
            "repository_connection_id",
        )
        .map_err(|e| context.validation(&e))?;
        Ok(context.response(
            self.state
                .inspect_ayni(&request.get_ref().repository_connection_id)
                .await
                .map_err(onboarding_error)?,
        ))
    }

    async fn list_runtime_profiles(
        &self,
        request: Request<v1::ListRuntimeProfilesRequest>,
    ) -> Result<Response<v1::ListRuntimeProfilesResponse>, Status> {
        let context = self.context(&request, "ListRuntimeProfiles")?;
        let value = request.get_ref();
        validate_optional_id(
            value.repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            api_validation::enum_filter(
                value.kind,
                "kind",
                &[
                    v1::RuntimeProfileKind::AyniManaged as i32,
                    v1::RuntimeProfileKind::CustomRuntime as i32,
                ],
            )
        })
        .and_then(|()| {
            api_validation::enum_filter(
                value.state,
                "state",
                &[
                    v1::RuntimeProfileState::SetupRequired as i32,
                    v1::RuntimeProfileState::SetupPrOpen as i32,
                    v1::RuntimeProfileState::AwaitingEnvironment as i32,
                    v1::RuntimeProfileState::Ready as i32,
                    v1::RuntimeProfileState::Updating as i32,
                    v1::RuntimeProfileState::Degraded as i32,
                    v1::RuntimeProfileState::Failed as i32,
                ],
            )
        })
        .and_then(|()| api_validation::page(value.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_runtime_profiles(
                message.repository_connection_id.as_deref(),
                message.kind,
                message.state,
                message.page.as_ref(),
            )
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRuntimeProfilesResponse {
            runtime_profiles: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_runtime_profile(
        &self,
        request: Request<v1::GetRuntimeProfileRequest>,
    ) -> Result<Response<v1::GetRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "GetRuntimeProfile")?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.runtime_profile(&message.runtime_profile_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRuntimeProfileResponse {
            runtime_profile: Some(resource),
        }))
    }

    async fn create_runtime_profile(
        &self,
        request: Request<v1::CreateRuntimeProfileRequest>,
    ) -> Result<Response<v1::CreateRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "CreateRuntimeProfile")?;
        api_validation::create_runtime_profile(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        if request
            .get_ref()
            .configuration
            .as_ref()
            .is_some_and(|configuration| {
                matches!(
                    configuration.configuration,
                    Some(v1::runtime_profile_configuration::Configuration::CustomRuntime(_))
                )
            })
        {
            return Err(context.unsupported_profile());
        }
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let (profile, revision) = with_store(&self.state, move |store| {
            store.create_runtime_profile(&key, &message)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CreateRuntimeProfileResponse {
            runtime_profile: Some(profile),
            revision: Some(revision),
        }))
    }

    async fn list_runtime_profile_revisions(
        &self,
        request: Request<v1::ListRuntimeProfileRevisionsRequest>,
    ) -> Result<Response<v1::ListRuntimeProfileRevisionsResponse>, Status> {
        let context = self.context(&request, "ListRuntimeProfileRevisions")?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_runtime_profile_revisions(&message.runtime_profile_id, message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRuntimeProfileRevisionsResponse {
            revisions: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_runtime_profile_revision(
        &self,
        request: Request<v1::GetRuntimeProfileRevisionRequest>,
    ) -> Result<Response<v1::GetRuntimeProfileRevisionResponse>, Status> {
        let context = self.context(&request, "GetRuntimeProfileRevision")?;
        validate_id(
            &request.get_ref().runtime_profile_revision_id,
            "runtime_profile_revision_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.runtime_profile_revision(&message.runtime_profile_revision_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRuntimeProfileRevisionResponse {
            revision: Some(resource),
        }))
    }

    async fn list_runtime_artifacts(
        &self,
        request: Request<v1::ListRuntimeArtifactsRequest>,
    ) -> Result<Response<v1::ListRuntimeArtifactsResponse>, Status> {
        let context = self.context(&request, "ListRuntimeArtifacts")?;
        let value = request.get_ref();
        validate_id(&value.runtime_profile_id, "runtime_profile_id")
            .and_then(|()| {
                validate_optional_id(
                    value.runtime_profile_revision_id.as_deref(),
                    "runtime_profile_revision_id",
                )
            })
            .and_then(|()| {
                api_validation::enum_filter(
                    value.status,
                    "status",
                    &[
                        v1::RuntimeArtifactStatus::Discovered as i32,
                        v1::RuntimeArtifactStatus::Verifying as i32,
                        v1::RuntimeArtifactStatus::Verified as i32,
                        v1::RuntimeArtifactStatus::Rejected as i32,
                        v1::RuntimeArtifactStatus::Unavailable as i32,
                        v1::RuntimeArtifactStatus::Failed as i32,
                    ],
                )
            })
            .and_then(|()| api_validation::page(value.page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_runtime_artifacts(
                &message.runtime_profile_id,
                message.runtime_profile_revision_id.as_deref(),
                message.status,
                message.page.as_ref(),
            )
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRuntimeArtifactsResponse {
            runtime_artifacts: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_runtime_artifact(
        &self,
        request: Request<v1::GetRuntimeArtifactRequest>,
    ) -> Result<Response<v1::GetRuntimeArtifactResponse>, Status> {
        let context = self.context(&request, "GetRuntimeArtifact")?;
        validate_id(
            &request.get_ref().runtime_artifact_id,
            "runtime_artifact_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.runtime_artifact(&message.runtime_artifact_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRuntimeArtifactResponse {
            runtime_artifact: Some(resource),
        }))
    }

    async fn list_automation_installations(
        &self,
        request: Request<v1::ListAutomationInstallationsRequest>,
    ) -> Result<Response<v1::ListAutomationInstallationsResponse>, Status> {
        let context = self.context(&request, "ListAutomationInstallations")?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, move |store| {
            store.list_automation_installations(&message.runtime_profile_id, message.page.as_ref())
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListAutomationInstallationsResponse {
            automation_installations: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_automation_installation(
        &self,
        request: Request<v1::GetAutomationInstallationRequest>,
    ) -> Result<Response<v1::GetAutomationInstallationResponse>, Status> {
        let context = self.context(&request, "GetAutomationInstallation")?;
        validate_id(
            &request.get_ref().automation_installation_id,
            "automation_installation_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.automation_installation(&message.automation_installation_id)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetAutomationInstallationResponse {
            automation_installation: Some(resource),
        }))
    }

    async fn install_repository_automation(
        &self,
        request: Request<v1::InstallRepositoryAutomationRequest>,
    ) -> Result<Response<v1::InstallRepositoryAutomationResponse>, Status> {
        let context = self.context(&request, "InstallRepositoryAutomation")?;
        let _ = request;
        Err(context.unsupported_profile())
    }

    async fn reconcile_runtime_profile(
        &self,
        request: Request<v1::ReconcileRuntimeProfileRequest>,
    ) -> Result<Response<v1::ReconcileRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "ReconcileRuntimeProfile")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, move |store| {
            store.reconcile_runtime_profile(&key, &message)
        })
        .await
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ReconcileRuntimeProfileResponse {
            runtime_profile: Some(resource),
        }))
    }

    type WatchRuntimeProfilesStream = BoxGrpcStream<v1::WatchRuntimeProfilesResponse>;

    async fn watch_runtime_profiles(
        &self,
        request: Request<v1::WatchRuntimeProfilesRequest>,
    ) -> Result<Response<Self::WatchRuntimeProfilesStream>, Status> {
        let context = self.context(&request, "WatchRuntimeProfiles")?;
        validate_optional_id(
            request.get_ref().repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            validate_optional_id(
                request.get_ref().runtime_profile_id.as_deref(),
                "runtime_profile_id",
            )
        })
        .map_err(|error| context.validation(&error))?;
        let stream = profile_watch(
            self.state.clone(),
            self.shutdown.clone(),
            request.into_inner(),
            context.request_id.clone(),
        );
        Ok(context.response(Box::pin(stream)))
    }
}
