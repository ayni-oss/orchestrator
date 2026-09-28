use super::*;

use crate::grpc_access::MAX_METADATA_BYTES;

#[test]
fn application_errors_use_rich_status_details() -> Result<(), prost::DecodeError> {
    let status = application_status(
        ApplicationError {
            reason: ApplicationErrorReason::ResourceNotFound,
            message: "development task was not found".to_owned(),
            resource_type: Some("development_task"),
            resource_id: Some("task-id".to_owned()),
        },
        Some("grpc-test"),
    );
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(
        status
            .metadata()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("grpc-test")
    );
    let rich = RichStatus::decode(status.details())?;
    assert_eq!(rich.code, Code::NotFound as i32);
    assert_eq!(rich.details.len(), 1);
    let detail = v1::ErrorDetail::decode(rich.details[0].value.as_slice())?;
    assert_eq!(detail.reason, v1::ErrorReason::ResourceNotFound as i32);
    assert_eq!(detail.request_id.as_deref(), Some("grpc-test"));
    Ok(())
}

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
    let first = context("client-a", "SaveRepository").idempotency_key("same-key".to_owned())?;
    let other_client =
        context("client-b", "SaveRepository").idempotency_key("same-key".to_owned())?;
    let other_rpc =
        context("client-a", "PrepareRepository").idempotency_key("same-key".to_owned())?;
    assert_ne!(first, other_client);
    assert_ne!(first, other_rpc);
    Ok(())
}

fn repository_enrollment(id: &str) -> v1::RepositoryEnrollment {
    v1::RepositoryEnrollment {
        id: id.to_owned(),
        owner: "ayni-oss".to_owned(),
        repository: "sandbox".to_owned(),
        installation_id: 42,
        base_branch: "main".to_owned(),
        lock_sha256: "a".repeat(64),
    }
}

#[tokio::test]
async fn native_server_exposes_health_reflection_and_product_rpc()
-> Result<(), Box<dyn std::error::Error>> {
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
    let response = health
        .check(HealthCheckRequest {
            service: "ayni.orchestrator.v1.RepositoryService".to_owned(),
        })
        .await?
        .into_inner();
    assert_eq!(
        response.status,
        tonic_health::pb::health_check_response::ServingStatus::Serving as i32
    );

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
    assert!(
        services
            .service
            .iter()
            .any(|service| { service.name == "ayni.orchestrator.v1.DevelopmentTaskService" })
    );
    let health_descriptor = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::FileContainingSymbol(
            "grpc.health.v1.Health".to_owned(),
        )),
    };
    let mut responses = reflection
        .server_reflection_info(tokio_stream::iter([health_descriptor]))
        .await?
        .into_inner();
    let response = responses
        .message()
        .await?
        .ok_or("reflection omitted the health descriptor")?;
    assert!(matches!(
        response.message_response,
        Some(MessageResponse::FileDescriptorResponse(_))
    ));

    shutdown_sender.send(true)?;
    server.await??;
    Ok(())
}

#[tokio::test]
async fn repository_rpc_propagates_correlation_and_rich_idempotency_error()
-> Result<(), Box<dyn std::error::Error>> {
    use v1::repository_service_server::RepositoryService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?;
    let (_shutdown_sender, shutdown) = watch::channel(false);
    let api = GrpcApi::new(state.clone(), shutdown);
    let mut request = Request::new(v1::SaveRepositoryRequest {
        idempotency_key: "repository-rpc".to_owned(),
        repository: Some(repository_enrollment("trial")),
    });
    request
        .metadata_mut()
        .insert("x-request-id", "repository-test".parse()?);
    let response = api.save_repository(request).await?;
    assert_eq!(
        response
            .metadata()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("repository-test")
    );
    assert_eq!(
        response
            .into_inner()
            .repository
            .and_then(|value| value.enrollment)
            .map(|value| value.id),
        Some("trial".to_owned())
    );

    let mut changed = repository_enrollment("trial");
    changed.base_branch = "release".to_owned();
    let Err(status) = api
        .save_repository(Request::new(v1::SaveRepositoryRequest {
            idempotency_key: "repository-rpc".to_owned(),
            repository: Some(changed),
        }))
        .await
    else {
        return Err("changed content reused an idempotency key".into());
    };
    assert_eq!(status.code(), Code::AlreadyExists);
    let rich = RichStatus::decode(status.details())?;
    let detail = v1::ErrorDetail::decode(rich.details[0].value.as_slice())?;
    assert_eq!(detail.reason, v1::ErrorReason::IdempotencyKeyReused as i32);
    Ok(())
}

async fn assert_watch_deadlines(
    api: &GrpcApi,
    task_id: &str,
    resource_version: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;
    use v1::development_task_service_server::DevelopmentTaskService;

    let mut deadline_request = Request::new(v1::WatchDevelopmentTaskRequest {
        task_id: task_id.to_owned(),
        after_resource_version: Some(resource_version),
    });
    deadline_request
        .metadata_mut()
        .insert("grpc-timeout", "20m".parse()?);
    let mut deadline_stream = api
        .watch_development_task(deadline_request)
        .await?
        .into_inner();
    let deadline_result =
        tokio::time::timeout(std::time::Duration::from_secs(1), deadline_stream.next())
            .await?
            .ok_or("deadline watch ended without a status")?;
    let Err(deadline_error) = deadline_result else {
        return Err("deadline watch returned a task update".into());
    };
    assert_eq!(deadline_error.code(), Code::DeadlineExceeded);

    let mut buffered_request = Request::new(v1::WatchDevelopmentTaskRequest {
        task_id: task_id.to_owned(),
        after_resource_version: None,
    });
    buffered_request
        .metadata_mut()
        .insert("grpc-timeout", "20m".parse()?);
    let mut buffered_stream = api
        .watch_development_task(buffered_request)
        .await?
        .into_inner();
    let initial = buffered_stream
        .next()
        .await
        .ok_or("deadline stream omitted its initial snapshot")??;
    assert_eq!(initial.resource_version, resource_version);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let deadline_result =
        tokio::time::timeout(std::time::Duration::from_secs(1), buffered_stream.next())
            .await?
            .ok_or("buffered deadline stream ended without a status")?;
    let Err(deadline_error) = deadline_result else {
        return Err("buffered deadline stream returned another update".into());
    };
    assert_eq!(deadline_error.code(), Code::DeadlineExceeded);
    Ok(())
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One end-to-end lifecycle test keeps the frozen values correlated.
async fn task_rpcs_preserve_frozen_input_and_resume_watch() -> Result<(), Box<dyn std::error::Error>>
{
    use tokio_stream::StreamExt as _;
    use v1::development_task_service_server::DevelopmentTaskService;

    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("orchestrator.sqlite"), 3)?
        .with_model_catalog(crate::ModelCatalog::fixed(vec![crate::AvailableModel {
            id: "gpt-5".to_owned(),
            name: Some("GPT-5".to_owned()),
            owned_by: Some("openai".to_owned()),
            created_at: None,
            context_length: None,
            max_output_tokens: None,
        }]));
    let application = Application::new(state.clone());
    application.save_repository(
        "save-trial".to_owned(),
        &RepositoryBinding {
            id: "trial".to_owned(),
            owner: "ayni-oss".to_owned(),
            repository: "sandbox".to_owned(),
            installation_id: 42,
            base_branch: "main".to_owned(),
            lock_sha256: "a".repeat(64),
        },
    )?;
    let prepared =
        application.prepare_repository("prepare-trial".to_owned(), "trial", "b".repeat(40))?;
    {
        let store = state
            .store
            .lock()
            .map_err(|_| io::Error::other("store lock was poisoned"))?;
        assert!(store.set_preparation_status(
            &prepared.id,
            PreparationStatus::Queued,
            PreparationStatus::Preparing,
        )?);
        assert!(store.complete_preparation(&prepared.id, 1)?);
        drop(store);
    }

    let (_shutdown_sender, shutdown) = watch::channel(false);
    let api = GrpcApi::new(state.clone(), shutdown);
    let response = api
        .create_development_task(Request::new(v1::CreateDevelopmentTaskRequest {
            idempotency_key: "create-task".to_owned(),
            input: Some(v1::DevelopmentTaskInput {
                repository_id: "trial".to_owned(),
                model: "gpt-5".to_owned(),
                start_revision: "b".repeat(40),
                request: "Correct the retry formula".to_owned(),
                acceptance_criteria: vec!["3 becomes 8".to_owned()],
                change_constraints: Some(v1::ChangeConstraints {
                    protected_paths: vec!["docs".to_owned()],
                    allow_test_edits: false,
                }),
                deadline: Some(Duration {
                    seconds: 3600,
                    nanos: 0,
                }),
            }),
        }))
        .await?
        .into_inner();
    let task = response.task.ok_or("create response omitted task")?;
    let task_id = task.id.clone();
    assert_eq!(task.resource_version, 1);
    assert_eq!(
        task.input
            .as_ref()
            .and_then(|input| input.change_constraints.as_ref())
            .map(|constraints| constraints.protected_paths.as_slice()),
        Some(["docs".to_owned()].as_slice())
    );

    // A pre-method-scoping action owned by another RPC must not reserve the
    // caller's key for CreateDevelopmentTask.
    {
        let store = state
            .store
            .lock()
            .map_err(|_| io::Error::other("store lock was poisoned"))?;
        store.connection.execute(
            "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES ('legacy-cross-method', ?1, 'cancel', 1)",
            [task_id.clone()],
        )?;
    }
    let cross_method = api
        .create_development_task(Request::new(v1::CreateDevelopmentTaskRequest {
            idempotency_key: "legacy-cross-method".to_owned(),
            input: task.input.clone(),
        }))
        .await?
        .into_inner()
        .task
        .ok_or("cross-method create response omitted task")?;
    assert_ne!(cross_method.id, task_id);

    assert_watch_deadlines(&api, &task_id, task.resource_version).await?;

    let mut resumed_stream = api
        .watch_development_task(Request::new(v1::WatchDevelopmentTaskRequest {
            task_id: task_id.clone(),
            after_resource_version: Some(task.resource_version),
        }))
        .await?
        .into_inner();
    let first_list_page = api
        .list_development_tasks(Request::new(v1::ListDevelopmentTasksRequest {
            page: Some(v1::PageRequest {
                page_size: 1,
                page_token: String::new(),
            }),
            phase: Some(v1::DevelopmentTaskPhase::Queued as i32),
        }))
        .await?
        .into_inner();
    assert_eq!(first_list_page.tasks.len(), 1);
    assert_eq!(first_list_page.tasks[0].id, cross_method.id);
    let list_token = first_list_page
        .page
        .ok_or("first list page omitted its cursor")?
        .next_page_token;
    assert!(!list_token.is_empty());
    let cancelled = api
        .cancel_development_task(Request::new(v1::CancelDevelopmentTaskRequest {
            idempotency_key: "create-task".to_owned(),
            task_id: task_id.clone(),
        }))
        .await?
        .into_inner()
        .task
        .ok_or("cancel response omitted task")?;
    assert_eq!(cancelled.phase, v1::DevelopmentTaskPhase::Cancelled as i32);
    assert!(cancelled.resource_version > task.resource_version);
    let resumed = tokio::time::timeout(std::time::Duration::from_secs(1), resumed_stream.next())
        .await?
        .ok_or("resumed watch ended before the next mutation")??;
    assert_eq!(resumed.resource_version, cancelled.resource_version);
    assert_eq!(
        resumed.task.map(|task| task.phase),
        Some(v1::DevelopmentTaskPhase::Cancelled as i32)
    );
    let second_list_page = api
        .list_development_tasks(Request::new(v1::ListDevelopmentTasksRequest {
            page: Some(v1::PageRequest {
                page_size: 1,
                page_token: list_token,
            }),
            phase: Some(v1::DevelopmentTaskPhase::Queued as i32),
        }))
        .await?
        .into_inner();
    assert_eq!(second_list_page.tasks.len(), 1);
    assert_eq!(second_list_page.tasks[0].id, task_id);
    assert_eq!(
        second_list_page.tasks[0].phase,
        v1::DevelopmentTaskPhase::Queued as i32
    );
    let replay = api
        .cancel_development_task(Request::new(v1::CancelDevelopmentTaskRequest {
            idempotency_key: "create-task".to_owned(),
            task_id: task_id.clone(),
        }))
        .await?
        .into_inner()
        .task
        .ok_or("cancel replay omitted task")?;
    assert_eq!(replay.resource_version, cancelled.resource_version);

    let mut stream = api
        .watch_development_task(Request::new(v1::WatchDevelopmentTaskRequest {
            task_id,
            after_resource_version: None,
        }))
        .await?
        .into_inner();
    let update = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await?
        .ok_or("watch stream ended before current snapshot")??;
    assert_eq!(update.resource_version, cancelled.resource_version);
    assert_eq!(
        update.task.map(|task| task.phase),
        Some(v1::DevelopmentTaskPhase::Cancelled as i32)
    );
    Ok(())
}

fn authenticated_request<T>(message: T, token: Option<&str>) -> Result<Request<T>, Status> {
    let mut request = Request::new(message);
    request.set_timeout(std::time::Duration::from_secs(10));
    if let Some(token) = token {
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| Status::internal("test credential was invalid"))?,
        );
    }
    Ok(request)
}

const PROTECTED_READ_TOKEN: &str = "read-client-token-12345678901234567890";
const PROTECTED_WRITE_A_TOKEN: &str = "write-a-client-token-123456789012345";
const PROTECTED_WRITE_B_TOKEN: &str = "write-b-client-token-123456789012345";
const PROTECTED_FORBIDDEN_TOKEN: &str = "forbidden-client-token-12345678901234";
const PROTECTED_EXPIRED_TOKEN: &str = "expired-client-token-1234567890123456";
const PROTECTED_TASK_TOKEN: &str = "task-client-token-12345678901234567890";

fn protected_clients() -> Result<Vec<u8>, serde_json::Error> {
    let digest = |token: &str| format!("{:x}", Sha256::digest(token.as_bytes()));
    serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "clients": [
            {"id":"reader", "token_sha256":digest(PROTECTED_READ_TOKEN),
             "expires_at_unix":u64::MAX, "capabilities":["provider_models.read"]},
            {"id":"writer-a", "token_sha256":digest(PROTECTED_WRITE_A_TOKEN),
             "expires_at_unix":u64::MAX, "capabilities":["repositories.write"]},
            {"id":"writer-b", "token_sha256":digest(PROTECTED_WRITE_B_TOKEN),
             "expires_at_unix":u64::MAX, "capabilities":["repositories.write"]},
            {"id":"forbidden", "token_sha256":digest(PROTECTED_FORBIDDEN_TOKEN),
             "expires_at_unix":u64::MAX, "capabilities":["tasks.read"]},
            {"id":"expired", "token_sha256":digest(PROTECTED_EXPIRED_TOKEN),
             "expires_at_unix":1, "capabilities":["provider_models.read"]},
            {"id":"task-client", "token_sha256":digest(PROTECTED_TASK_TOKEN),
             "expires_at_unix":u64::MAX,
             "capabilities":["tasks.read","tasks.create","tasks.cancel","publications.create"]}
        ]
    }))
}

async fn assert_protected_provider_access(
    channel: tonic::transport::Channel,
) -> Result<(), Box<dyn std::error::Error>> {
    use tonic_health::pb::{HealthCheckRequest, health_client::HealthClient};

    let mut health = HealthClient::new(channel.clone());
    let Err(health_status) = health
        .check(HealthCheckRequest {
            service: "ayni.orchestrator.v1.RepositoryService".to_owned(),
        })
        .await
    else {
        return Err("protected listener exposed an unauthenticated health service".into());
    };
    assert_eq!(health_status.code(), Code::Unimplemented);

    let mut provider = v1::provider_service_client::ProviderServiceClient::new(channel);
    let response = provider
        .list_provider_models(authenticated_request(
            v1::ListProviderModelsRequest { page: None },
            Some(PROTECTED_READ_TOKEN),
        )?)
        .await?;
    assert!(
        response
            .metadata()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .is_some_and(valid_request_id)
    );
    assert_eq!(response.into_inner().models.len(), 1);

    let Err(missing) = provider
        .list_provider_models(authenticated_request(
            v1::ListProviderModelsRequest { page: None },
            None,
        )?)
        .await
    else {
        return Err("missing credential was accepted".into());
    };
    assert_eq!(missing.code(), Code::Unauthenticated);
    assert!(
        missing
            .metadata()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .is_some_and(valid_request_id)
    );

    let mut duplicate_credential = authenticated_request(
        v1::ListProviderModelsRequest { page: None },
        Some(PROTECTED_READ_TOKEN),
    )?;
    duplicate_credential.metadata_mut().append(
        "authorization",
        format!("Bearer {PROTECTED_READ_TOKEN}").parse()?,
    );
    let Err(duplicate_status) = provider.list_provider_models(duplicate_credential).await else {
        return Err("duplicate credentials were accepted".into());
    };
    assert_eq!(duplicate_status.code(), Code::Unauthenticated);

    let mut oversized_metadata = authenticated_request(
        v1::ListProviderModelsRequest { page: None },
        Some(PROTECTED_READ_TOKEN),
    )?;
    oversized_metadata
        .metadata_mut()
        .insert("x-padding", "x".repeat(MAX_METADATA_BYTES).parse()?);
    let Err(metadata_status) = provider.list_provider_models(oversized_metadata).await else {
        return Err("oversized metadata was accepted".into());
    };
    assert_eq!(metadata_status.code(), Code::ResourceExhausted);

    let Err(forbidden) = provider
        .list_provider_models(authenticated_request(
            v1::ListProviderModelsRequest { page: None },
            Some(PROTECTED_FORBIDDEN_TOKEN),
        )?)
        .await
    else {
        return Err("under-scoped credential was accepted".into());
    };
    assert_eq!(forbidden.code(), Code::PermissionDenied);
    let Err(expired) = provider
        .list_provider_models(authenticated_request(
            v1::ListProviderModelsRequest { page: None },
            Some(PROTECTED_EXPIRED_TOKEN),
        )?)
        .await
    else {
        return Err("expired credential was accepted".into());
    };
    assert_eq!(expired.code(), Code::Unauthenticated);
    Ok(())
}

async fn assert_protected_task_lifecycle(
    channel: tonic::transport::Channel,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tasks = v1::development_task_service_client::DevelopmentTaskServiceClient::new(channel);
    let created = tasks
        .create_development_task(authenticated_request(
            v1::CreateDevelopmentTaskRequest {
                idempotency_key: "protected-create".to_owned(),
                input: Some(v1::DevelopmentTaskInput {
                    repository_id: "protected-trial".to_owned(),
                    model: "gpt-5".to_owned(),
                    start_revision: "b".repeat(40),
                    request: "Exercise the protected task lifecycle".to_owned(),
                    acceptance_criteria: vec!["The protected lifecycle completes".to_owned()],
                    change_constraints: Some(v1::ChangeConstraints {
                        protected_paths: Vec::new(),
                        allow_test_edits: true,
                    }),
                    deadline: Some(Duration {
                        seconds: 3_600,
                        nanos: 0,
                    }),
                }),
            },
            Some(PROTECTED_TASK_TOKEN),
        )?)
        .await?
        .into_inner()
        .task
        .ok_or("protected create response omitted task")?;

    let mut too_long = authenticated_request(
        v1::WatchDevelopmentTaskRequest {
            task_id: created.id.clone(),
            after_resource_version: Some(created.resource_version),
        },
        Some(PROTECTED_TASK_TOKEN),
    )?;
    too_long.set_timeout(std::time::Duration::from_secs(301));
    let Err(too_long_status) = tasks.watch_development_task(too_long).await else {
        return Err("overlong protected watch deadline was accepted".into());
    };
    assert_eq!(too_long_status.code(), Code::InvalidArgument);

    let mut expiring = authenticated_request(
        v1::WatchDevelopmentTaskRequest {
            task_id: created.id.clone(),
            after_resource_version: Some(created.resource_version),
        },
        Some(PROTECTED_TASK_TOKEN),
    )?;
    expiring.set_timeout(std::time::Duration::from_millis(20));
    let mut expiring_stream = tasks.watch_development_task(expiring).await?.into_inner();
    let deadline_result =
        tokio::time::timeout(std::time::Duration::from_secs(1), expiring_stream.message()).await?;
    let Err(deadline_status) = deadline_result else {
        return Err("protected watch did not enforce its deadline".into());
    };
    assert_eq!(deadline_status.code(), Code::DeadlineExceeded);

    let mut resumed_stream = tasks
        .watch_development_task(authenticated_request(
            v1::WatchDevelopmentTaskRequest {
                task_id: created.id.clone(),
                after_resource_version: Some(created.resource_version),
            },
            Some(PROTECTED_TASK_TOKEN),
        )?)
        .await?
        .into_inner();
    let cancelled = tasks
        .cancel_development_task(authenticated_request(
            v1::CancelDevelopmentTaskRequest {
                idempotency_key: "protected-cancel".to_owned(),
                task_id: created.id.clone(),
            },
            Some(PROTECTED_TASK_TOKEN),
        )?)
        .await?
        .into_inner()
        .task
        .ok_or("protected cancel response omitted task")?;
    assert_eq!(cancelled.phase, v1::DevelopmentTaskPhase::Cancelled as i32);
    let resumed = tokio::time::timeout(std::time::Duration::from_secs(1), resumed_stream.message())
        .await??
        .ok_or("protected resumed watch ended before cancellation")?;
    assert_eq!(resumed.resource_version, cancelled.resource_version);
    assert_eq!(
        resumed.task.map(|task| task.phase),
        Some(v1::DevelopmentTaskPhase::Cancelled as i32)
    );
    Ok(())
}

async fn assert_protected_repository_boundaries(
    channel: tonic::transport::Channel,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut repositories = v1::repository_service_client::RepositoryServiceClient::new(channel)
        .max_encoding_message_size(3 * 1024 * 1024);
    let mut first = repository_enrollment("principal-scope");
    first.base_branch = "main".to_owned();
    repositories
        .save_repository(authenticated_request(
            v1::SaveRepositoryRequest {
                idempotency_key: "shared-caller-key".to_owned(),
                repository: Some(first),
            },
            Some(PROTECTED_WRITE_A_TOKEN),
        )?)
        .await?;
    let mut second = repository_enrollment("principal-scope");
    second.base_branch = "release".to_owned();
    let response = repositories
        .save_repository(authenticated_request(
            v1::SaveRepositoryRequest {
                idempotency_key: "shared-caller-key".to_owned(),
                repository: Some(second),
            },
            Some(PROTECTED_WRITE_B_TOKEN),
        )?)
        .await?
        .into_inner();
    assert_eq!(
        response
            .repository
            .and_then(|repository| repository.enrollment)
            .map(|repository| repository.base_branch),
        Some("release".to_owned())
    );

    let Err(invalid_key) = repositories
        .save_repository(authenticated_request(
            v1::SaveRepositoryRequest {
                idempotency_key: String::new(),
                repository: Some(repository_enrollment("invalid-key")),
            },
            Some(PROTECTED_WRITE_A_TOKEN),
        )?)
        .await
    else {
        return Err("invalid idempotency key was accepted".into());
    };
    assert_eq!(invalid_key.code(), Code::InvalidArgument);

    let Err(oversized) = repositories
        .save_repository(authenticated_request(
            v1::SaveRepositoryRequest {
                idempotency_key: "oversized-request".to_owned(),
                repository: Some(v1::RepositoryEnrollment {
                    owner: "x".repeat(MAX_REQUEST_BYTES + 1),
                    ..repository_enrollment("oversized")
                }),
            },
            Some(PROTECTED_WRITE_A_TOKEN),
        )?)
        .await
    else {
        return Err("oversized request was accepted".into());
    };
    assert_eq!(oversized.code(), Code::OutOfRange);
    Ok(())
}

#[tokio::test]
async fn protected_tls_path_enforces_identity_capabilities_expiry_and_size()
-> Result<(), Box<dyn std::error::Error>> {
    use rcgen::{CertifiedKey, generate_simple_self_signed};
    use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_owned()])?;
    let certificate_pem = cert.pem();
    let access = CustomClientAccess::from_parts_for_test(
        certificate_pem.as_bytes().to_vec(),
        signing_key.serialize_pem().into_bytes(),
        &protected_clients()?,
    )?;
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
    let application = Application::new(state.clone());
    application.save_repository(
        "protected-save".to_owned(),
        &RepositoryBinding {
            id: "protected-trial".to_owned(),
            owner: "ayni-oss".to_owned(),
            repository: "sandbox".to_owned(),
            installation_id: 42,
            base_branch: "main".to_owned(),
            lock_sha256: "a".repeat(64),
        },
    )?;
    let prepared = application.prepare_repository(
        "protected-prepare".to_owned(),
        "protected-trial",
        "b".repeat(40),
    )?;
    {
        let store = state
            .store
            .lock()
            .map_err(|_| io::Error::other("store lock was poisoned"))?;
        assert!(store.set_preparation_status(
            &prepared.id,
            PreparationStatus::Queued,
            PreparationStatus::Preparing,
        )?);
        assert!(store.complete_preparation(&prepared.id, 1)?);
        drop(store);
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let (shutdown_sender, shutdown) = watch::channel(false);
    let server = tokio::spawn(serve_custom(address, state, shutdown, access));
    let endpoint = format!("https://{address}");
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(certificate_pem))
        .domain_name("localhost");
    let channel = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match Endpoint::from_shared(endpoint.clone())?
                .tls_config(tls.clone())?
                .connect()
                .await
            {
                Ok(channel) => break Ok::<_, tonic::transport::Error>(channel),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await??;

    assert_protected_provider_access(channel.clone()).await?;
    assert_protected_task_lifecycle(channel.clone()).await?;
    assert_protected_repository_boundaries(channel).await?;

    shutdown_sender.send(true)?;
    server.await??;
    Ok(())
}
