use super::*;
#[test]
pub(super) fn accepts_only_bounded_safe_request_ids() {
    assert!(valid_request_id("grpc-test_1.2"));
    assert!(!valid_request_id(""));
    assert!(!valid_request_id("contains spaces"));
    assert!(!valid_request_id(&"x".repeat(65)));
}

#[test]
pub(super) fn custom_idempotency_is_scoped_by_principal_and_rpc() -> Result<(), Status> {
    let context = |client_id: &str, rpc: &'static str| RequestContext {
        request_id: "scope-test".to_owned(),
        client_id: Some(client_id.to_owned()),
        rpc,
        started: Instant::now(),
    };
    let first = context("client-a", "CreateRepositoryConnection").idempotency_key("same-key")?;
    let other_client =
        context("client-b", "CreateRepositoryConnection").idempotency_key("same-key")?;
    let other_rpc = context("client-a", "CreateRuntimeProfile").idempotency_key("same-key")?;
    assert_ne!(first, other_client);
    assert_ne!(first, other_rpc);
    Ok(())
}

#[tokio::test]
pub(super) async fn native_server_exposes_clean_services_and_provider_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    use tonic_health::pb::{HealthCheckRequest, health_client::HealthClient};
    use tonic_reflection::pb::v1::{
        ServerReflectionRequest, server_reflection_client::ServerReflectionClient,
        server_reflection_request::MessageRequest, server_reflection_response::MessageResponse,
    };

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?
        .with_model_catalog(crate::ModelCatalog::fixed(vec![crate::AvailableModel {
            id: "gpt-5.6-sol".to_owned(),
            name: None,
            owned_by: Some("openai".to_owned()),
            created_at: None,
            context_length: None,
            max_output_tokens: None,
        }]));
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let (shutdown_sender, shutdown) = shutdown_watch::channel(false);
    let server = tokio::spawn(serve(address, state, shutdown));
    let endpoint = format!("http://{address}");
    let channel = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match tonic::transport::Endpoint::from_shared(endpoint.clone())?
                .connect()
                .await
            {
                Ok(channel) => break Ok::<_, tonic::transport::Error>(channel),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await??;

    let mut health = HealthClient::new(channel.clone());
    for service in [
        "ayni.orchestrator.v1.RepositoryService",
        "ayni.orchestrator.v1.RuntimeProfileService",
        "ayni.orchestrator.v1.DevelopmentTaskService",
    ] {
        let response = health
            .check(HealthCheckRequest {
                service: service.to_owned(),
            })
            .await?
            .into_inner();
        assert_eq!(
            response.status,
            tonic_health::pb::health_check_response::ServingStatus::Serving as i32
        );
    }

    let mut provider = v1::provider_service_client::ProviderServiceClient::new(channel.clone());
    let models = provider
        .list_provider_models(v1::ListProviderModelsRequest {
            page: None,
            connection_id: String::new(),
        })
        .await?
        .into_inner();
    assert_eq!(models.models.len(), 1);
    assert_eq!(models.models[0].id, "gpt-5.6-sol");

    let mut reflection = ServerReflectionClient::new(channel);
    let request = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::ListServices(String::new())),
    };
    let mut responses = reflection
        .server_reflection_info(tokio_stream::iter([request]))
        .await?
        .into_inner();
    let response = responses
        .message()
        .await?
        .ok_or("reflection returned no response")?;
    let Some(MessageResponse::ListServicesResponse(services)) = response.message_response else {
        return Err("reflection returned an unexpected response".into());
    };
    let names = services
        .service
        .into_iter()
        .map(|service| service.name)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("ayni.orchestrator.v1.RuntimeProfileService"));

    shutdown_sender.send(true)?;
    server.await??;
    Ok(())
}

#[tokio::test]
pub(super) async fn server_validates_bounds_before_durable_resource_operations()
-> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = shutdown_watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let Err(status) = api
        .create_repository_connection(Request::new(v1::CreateRepositoryConnectionRequest {
            use_ayni: None,
            idempotency_key: String::new(),
            provider: None,
            display_name: String::new(),
            default_branch: String::new(),
            registry_connection_id: String::new(),
            image_namespace: String::new(),
        }))
        .await
    else {
        return Err("invalid request must fail".into());
    };
    assert_eq!(status.code(), Code::InvalidArgument);
    let rich = RichStatus::decode(status.details())?;
    let detail = v1::ErrorDetail::decode(rich.details[0].value.as_slice())?;
    assert_eq!(detail.reason, v1::ErrorReason::InvalidIdentifier as i32);
    assert_eq!(detail.metadata[0].key, "field");
    assert_eq!(detail.metadata[0].value, "idempotency_key");
    Ok(())
}

#[tokio::test]
pub(super) async fn unknown_request_enums_fail_closed_with_a_stable_reason()
-> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = shutdown_watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let Err(status) = api
        .list_repository_connections(Request::new(v1::ListRepositoryConnectionsRequest {
            page: None,
            status: Some(999),
        }))
        .await
    else {
        return Err("unknown enum value must fail".into());
    };
    assert_eq!(status.code(), Code::InvalidArgument);
    let rich = RichStatus::decode(status.details())?;
    let detail = v1::ErrorDetail::decode(rich.details[0].value.as_slice())?;
    assert_eq!(detail.reason, v1::ErrorReason::UnknownEnumValue as i32);
    assert_eq!(detail.metadata[0].value, "status");
    Ok(())
}

#[tokio::test]
pub(super) async fn clean_v1_repository_operations_are_durable_and_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let database = directory.path().join("orchestrator.sqlite");
    let state = AppState::open(&database, 3)?;
    let (_shutdown_sender, shutdown) = shutdown_watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let request = v1::CreateRepositoryConnectionRequest {
        use_ayni: None,
        idempotency_key: "repository:create:1".to_owned(),
        provider: Some(v1::RepositoryProvider {
            provider: Some(v1::repository_provider::Provider::Github(
                v1::GithubRepository {
                    repository_id: "123".to_owned(),
                    owner: "ayni-oss".to_owned(),
                    name: "sandbox".to_owned(),
                    installation_id: 42,
                    html_url: "https://github.com/ayni-oss/sandbox".to_owned(),
                },
            )),
        }),
        display_name: "Sandbox".to_owned(),
        default_branch: "main".to_owned(),
        registry_connection_id: String::new(),
        image_namespace: String::new(),
    };
    let first = api
        .create_repository_connection(Request::new(request.clone()))
        .await?
        .into_inner()
        .repository_connection
        .ok_or("missing connection")?;
    let replay = api
        .create_repository_connection(Request::new(request))
        .await?
        .into_inner()
        .repository_connection
        .ok_or("missing replay")?;
    assert_eq!(first.id, replay.id);
    drop(api);

    let reopened = AppState::open(database, 3)?;
    let (_shutdown_sender, shutdown) = shutdown_watch::channel(false);
    let api = GrpcApi::new(reopened, shutdown);
    let listed = api
        .list_repository_connections(Request::new(v1::ListRepositoryConnectionsRequest {
            page: None,
            status: None,
        }))
        .await?
        .into_inner();
    assert_eq!(listed.repository_connections.len(), 1);
    assert!(listed.page.is_some_and(|page| page.snapshot_cursor > 0));
    Ok(())
}

#[tokio::test]
pub(super) async fn custom_runtime_is_explicitly_unsupported_in_the_ayni_first_release()
-> Result<(), Box<dyn std::error::Error>> {
    use v1::runtime_profile_service_server::RuntimeProfileService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = shutdown_watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let Err(status) = api
        .create_runtime_profile(Request::new(v1::CreateRuntimeProfileRequest {
            idempotency_key: "custom:1".to_owned(),
            repository_connection_id: "repository:1".to_owned(),
            repository_snapshot_id: "snapshot:1".to_owned(),
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: Some(
                    v1::runtime_profile_configuration::Configuration::CustomRuntime(
                        v1::CustomRuntimeProfileRevision {
                            image_reference: "registry.example/runtime:stable".to_owned(),
                        },
                    ),
                ),
            }),
        }))
        .await
    else {
        return Err("Custom Runtime must be explicitly unsupported".into());
    };
    assert_eq!(status.code(), Code::FailedPrecondition);
    let rich = RichStatus::decode(status.details())?;
    let detail = v1::ErrorDetail::decode(rich.details[0].value.as_slice())?;
    assert_eq!(detail.reason, v1::ErrorReason::ProfileUnsupported as i32);
    Ok(())
}

#[tokio::test]
pub(super) async fn project_api_validates_membership_versions_and_archive()
-> Result<(), Box<dyn std::error::Error>> {
    use v1::project_service_server::ProjectService;
    use v1::repository_service_server::RepositoryService;
    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("projects.sqlite"), 3)?;
    let (_sender, shutdown) = shutdown_watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let invalid = api
        .create_project(Request::new(v1::CreateProjectRequest {
            workspace_id: String::new(),
            goal: String::new(),
            idempotency_key: "project".into(),
            display_name: "   ".into(),
            repository_connection_id: "repository:1".into(),
            repository_connection_ids: vec![],
        }))
        .await;
    assert_eq!(
        invalid.err().ok_or("expected invalid input")?.code(),
        Code::InvalidArgument
    );
    let repo = api
        .create_repository_connection(Request::new(v1::CreateRepositoryConnectionRequest {
            use_ayni: None,
            idempotency_key: "repository".into(),
            display_name: "Sandbox".into(),
            default_branch: "main".into(),
            provider: Some(v1::RepositoryProvider {
                provider: Some(v1::repository_provider::Provider::Github(
                    v1::GithubRepository {
                        repository_id: "123".into(),
                        owner: "ayni-oss".into(),
                        name: "sandbox".into(),
                        installation_id: 42,
                        html_url: "https://github.com/ayni-oss/sandbox".into(),
                    },
                )),
            }),
            registry_connection_id: String::new(),
            image_namespace: String::new(),
        }))
        .await?
        .into_inner()
        .repository_connection
        .ok_or("missing repository")?;
    let project = api
        .create_project(Request::new(v1::CreateProjectRequest {
            workspace_id: String::new(),
            goal: String::new(),
            idempotency_key: "project".into(),
            display_name: "Sandbox".into(),
            repository_connection_id: repo.id,
            repository_connection_ids: vec![],
        }))
        .await?
        .into_inner()
        .project
        .ok_or("missing project")?;
    let rename = api
        .update_project(Request::new(v1::UpdateProjectRequest {
            goal: None,
            agent_defaults: None,
            idempotency_key: "rename".into(),
            project_id: project.id.clone(),
            display_name: "Renamed".into(),
            expected_resource_version: 2,
            repository_connection_ids: vec![],
        }))
        .await;
    assert_eq!(
        rename.err().ok_or("expected stale version")?.code(),
        Code::Aborted
    );
    let archived = api
        .archive_project(Request::new(v1::ArchiveProjectRequest {
            idempotency_key: "archive".into(),
            project_id: project.id.clone(),
            expected_resource_version: 1,
        }))
        .await?
        .into_inner()
        .project
        .ok_or("missing archive")?;
    assert_eq!(archived.state, v1::ProjectState::Archived as i32);
    let listed = api
        .list_projects(Request::new(v1::ListProjectsRequest {
            state: Some(v1::ProjectState::Archived as i32),
            page: None,
        }))
        .await?
        .into_inner();
    assert_eq!(listed.projects, vec![archived]);
    workspace_api_preserves_project_history(&api, &project).await?;
    Ok(())
}

async fn workspace_api_preserves_project_history(
    api: &GrpcApi,
    project: &v1::Project,
) -> Result<(), Box<dyn std::error::Error>> {
    use v1::project_service_server::ProjectService;
    let workspace = api
        .get_workspace(Request::new(v1::GetWorkspaceRequest {
            workspace_id: project.workspace_id.clone(),
        }))
        .await?
        .into_inner()
        .workspace
        .ok_or("missing workspace")?;
    let create = v1::CreateProjectRequest {
        idempotency_key: "goal-project".into(),
        display_name: "Goal project".into(),
        workspace_id: workspace.id.clone(),
        goal: "Ship a bounded improvement".into(),
        repository_connection_ids: workspace.repository_connection_ids.clone(),
        ..Default::default()
    };
    let child = api
        .create_project(Request::new(create.clone()))
        .await?
        .into_inner()
        .project
        .ok_or("missing child")?;
    assert_eq!(child.workspace_id, workspace.id);
    assert_eq!(child.goal, "Ship a bounded improvement");
    let update = v1::UpdateWorkspaceRequest {
        planner_idle_timeout_seconds: None,
        idempotency_key: "rename-workspace".into(),
        workspace_id: workspace.id.clone(),
        expected_resource_version: workspace.resource_version,
        display_name: "Renamed workspace".into(),
        repository_connection_ids: vec![],
        agent_defaults: None,
    };
    let renamed = api
        .update_workspace(Request::new(update.clone()))
        .await?
        .into_inner()
        .workspace
        .ok_or("missing renamed workspace")?;
    assert_eq!(
        api.update_workspace(Request::new(update))
            .await?
            .into_inner()
            .workspace,
        Some(renamed.clone())
    );
    let archived_workspace = api
        .archive_workspace(Request::new(v1::ArchiveWorkspaceRequest {
            idempotency_key: "archive-workspace".into(),
            workspace_id: workspace.id.clone(),
            expected_resource_version: renamed.resource_version,
        }))
        .await?
        .into_inner()
        .workspace
        .ok_or("missing archived workspace")?;
    assert_eq!(
        archived_workspace.state,
        v1::WorkspaceState::Archived as i32
    );
    assert_eq!(
        api.create_project(Request::new(create.clone()))
            .await?
            .into_inner()
            .project,
        Some(child.clone())
    );
    let mut new_create = create;
    new_create.idempotency_key = "new-after-archive".into();
    assert_eq!(
        api.create_project(Request::new(new_create))
            .await
            .err()
            .ok_or("must block archived workspace")?
            .code(),
        Code::FailedPrecondition
    );
    let listed = api
        .list_workspaces(Request::new(v1::ListWorkspacesRequest {
            state: Some(v1::WorkspaceState::Archived as i32),
            page: None,
        }))
        .await?
        .into_inner();
    assert_eq!(listed.workspaces, vec![archived_workspace]);
    assert_eq!(
        api.get_project(Request::new(v1::GetProjectRequest {
            project_id: child.id.clone()
        }))
        .await?
        .into_inner()
        .project,
        Some(child)
    );
    Ok(())
}
