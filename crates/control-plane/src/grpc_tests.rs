use super::*;

#[test]
fn accepts_only_bounded_safe_request_ids() {
    assert!(valid_request_id("grpc-test_1.2"));
    assert!(!valid_request_id(""));
    assert!(!valid_request_id("contains spaces"));
    assert!(!valid_request_id(&"x".repeat(65)));
}

#[test]
fn custom_idempotency_is_scoped_by_principal_and_rpc() -> Result<(), Status> {
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
async fn native_server_exposes_clean_services_and_provider_catalog(
) -> Result<(), Box<dyn std::error::Error>> {
    use tonic_health::pb::{HealthCheckRequest, health_client::HealthClient};
    use tonic_reflection::pb::v1::{
        ServerReflectionRequest, server_reflection_client::ServerReflectionClient,
        server_reflection_request::MessageRequest, server_reflection_response::MessageResponse,
    };

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?
        .with_model_catalog(crate::ModelCatalog::fixed(vec![crate::AvailableModel {
            id: "gpt-5".to_owned(),
            name: None,
            owned_by: Some("openai".to_owned()),
            created_at: None,
            context_length: None,
            max_output_tokens: None,
        }]));
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let (shutdown_sender, shutdown) = watch::channel(false);
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
        "ayni.orchestrator.v1.ReviewService",
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
        .list_provider_models(v1::ListProviderModelsRequest { page: None })
        .await?
        .into_inner();
    assert_eq!(models.models.len(), 1);
    assert_eq!(models.models[0].id, "gpt-5");

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
    assert!(names.contains("ayni.orchestrator.v1.ReviewService"));

    shutdown_sender.send(true)?;
    server.await??;
    Ok(())
}

#[tokio::test]
async fn server_validates_bounds_before_unimplemented_resource_operations(
) -> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let Err(status) = api
        .create_repository_connection(Request::new(v1::CreateRepositoryConnectionRequest {
            idempotency_key: String::new(),
            provider: None,
            display_name: String::new(),
            default_branch: String::new(),
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
async fn unknown_request_enums_fail_closed_with_a_stable_reason(
) -> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = watch::channel(false);
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
async fn pending_clean_v1_persistence_has_a_machine_readable_status(
) -> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = watch::channel(false);
    let api = GrpcApi::new(state, shutdown);
    let Err(status) = api
        .list_repository_connections(Request::new(v1::ListRepositoryConnectionsRequest {
            page: None,
            status: None,
        }))
        .await
    else {
        return Err("pending resource operation must not report success".into());
    };
    assert_eq!(status.code(), Code::Unimplemented);
    let rich = RichStatus::decode(status.details())?;
    let detail = v1::ErrorDetail::decode(rich.details[0].value.as_slice())?;
    assert_eq!(detail.reason, v1::ErrorReason::NotImplemented as i32);
    Ok(())
}

#[tokio::test]
async fn custom_runtime_is_explicitly_unsupported_in_the_ayni_first_release(
) -> Result<(), Box<dyn std::error::Error>> {
    use v1::runtime_profile_service_server::RuntimeProfileService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = watch::channel(false);
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
