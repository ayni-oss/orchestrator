#[path = "support/plan_acceptance.rs"]
mod plan_acceptance;

#[path = "support/console.rs"]
mod console;

use ayni_orchestrator_api::v1::{
    CreateConversationRequest, CreateProjectRequest, CreateWorkspaceRequest,
    GetConversationSnapshotRequest, ListConversationHistoryRequest, SendMessageRequest,
    WatchConversationRequest,
    project_conversation_service_client::ProjectConversationServiceClient,
    project_service_client::ProjectServiceClient,
};
use ayni_orchestrator_control_plane::{WorkflowReadiness, router, serve_grpc};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use http_body_util::BodyExt as _;
use std::{env, error::Error, io, net::SocketAddr, time::Duration};
use tokio::sync::watch;
use tonic::{Code, Request, transport::Channel};
use tower::ServiceExt as _;
use uuid::Uuid;

const TEST_URL_VARIABLE: &str = "AYNI_POSTGRES_FEASIBILITY_URL";
const TEST_ROLE_VARIABLE: &str = "AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE";
type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type TestServer = (
    SocketAddr,
    watch::Sender<bool>,
    tokio::task::JoinHandle<Result<(), io::Error>>,
);

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL gate"]
async fn trusted_listener_and_readiness_cover_private_workflow_boundary() -> TestResult {
    let database_url = env::var(TEST_URL_VARIABLE)
        .map_err(|_| io::Error::other(format!("{TEST_URL_VARIABLE} is required")))?;
    let runtime_role = env::var(TEST_ROLE_VARIABLE)
        .map_err(|_| io::Error::other(format!("{TEST_ROLE_VARIABLE} is required")))?;
    ayni_orchestrator_workflow_postgres::validate_for_runtime(&database_url, &runtime_role).await?;

    let readiness = router(WorkflowReadiness::new(database_url.clone(), runtime_role));
    let response = readiness
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/readyz")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let _ = response.into_body().collect().await?;
    let metrics = readiness
        .oneshot(
            axum::http::Request::builder()
                .uri("/metrics")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(metrics.status(), axum::http::StatusCode::OK);
    let metrics = String::from_utf8(metrics.into_body().collect().await?.to_bytes().to_vec())?;
    assert!(metrics.contains("workflow_database_ready 1"));
    assert!(metrics.contains("workflow_"));
    assert!(!metrics.contains("durable admission"));
    assert!(!metrics.contains("postgresql://"));

    let (address, shutdown, server) = start_server(&database_url)?;
    let channel = connect(address).await?;
    exercise_hierarchy_and_admission(channel).await?;
    console::exercise(address).await?;
    shutdown.send(true)?;
    server.await??;
    Ok(())
}

#[tokio::test]
async fn readiness_fails_closed_and_listener_rejects_public_bind() -> TestResult {
    let readiness = router(WorkflowReadiness::new(
        "postgresql://127.0.0.1:1/unavailable",
        "ayni_runtime",
    ));
    let response = readiness
        .oneshot(
            axum::http::Request::builder()
                .uri("/readyz")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );

    let (_sender, receiver) = watch::channel(false);
    let error = serve_grpc(
        "0.0.0.0:50051".parse()?,
        WorkflowStore::new("postgresql://127.0.0.1:1/unavailable"),
        receiver,
    )
    .await;
    assert!(error.is_err());
    prove_database_unavailable().await
}

async fn prove_database_unavailable() -> TestResult {
    let unavailable = "postgresql://127.0.0.1:1/unavailable";
    let (address, shutdown, server) = start_server(unavailable)?;
    let channel = connect(address).await?;
    let mut projects = ProjectServiceClient::new(channel.clone());
    let mut conversations = ProjectConversationServiceClient::new(channel);
    let nonce = Uuid::now_v7().to_string();
    let admission = projects
        .create_workspace(request(
            CreateWorkspaceRequest {
                idempotency_key: format!("unavailable-{nonce}"),
                display_name: "Unavailable database".to_owned(),
            },
            &nonce,
        ))
        .await
        .err()
        .ok_or("admission unexpectedly succeeded without PostgreSQL")?;
    assert_eq!(admission.code(), Code::Unavailable);
    let observation = conversations
        .get_conversation_snapshot(request(
            GetConversationSnapshotRequest {
                workspace_id: nonce.clone(),
                project_id: nonce.clone(),
                conversation_id: nonce.clone(),
            },
            &nonce,
        ))
        .await
        .err()
        .ok_or("observation unexpectedly succeeded without PostgreSQL")?;
    assert_eq!(observation.code(), Code::Unavailable);
    shutdown.send(true)?;
    server.await??;
    Ok(())
}

fn start_server(database_url: &str) -> TestResult<TestServer> {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = reservation.local_addr()?;
    drop(reservation);
    let (shutdown, receiver) = watch::channel(false);
    let store = WorkflowStore::new(database_url.to_owned());
    let server = tokio::spawn(serve_grpc(address, store, receiver));
    Ok((address, shutdown, server))
}

async fn connect(address: SocketAddr) -> TestResult<Channel> {
    let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?;
    for _ in 0..50 {
        if let Ok(channel) = endpoint.connect().await {
            return Ok(channel);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(io::Error::other("trusted workflow listener did not become ready").into())
}

async fn exercise_hierarchy_and_admission(channel: Channel) -> TestResult {
    let nonce = Uuid::now_v7().to_string();
    let mut projects = ProjectServiceClient::new(channel.clone());
    let mut conversations = ProjectConversationServiceClient::new(channel);
    let workspace_key = format!("grpc-workspace-{nonce}");
    let workspace_request = CreateWorkspaceRequest {
        idempotency_key: workspace_key.clone(),
        display_name: "gRPC workspace".to_owned(),
    };
    let created = projects
        .create_workspace(request(workspace_request.clone(), &nonce))
        .await?
        .into_inner();
    let workspace = created.workspace.ok_or("workspace missing")?;
    let receipt = created.receipt.ok_or("workspace receipt missing")?;
    let replay = projects
        .create_workspace(request(workspace_request, &nonce))
        .await?
        .into_inner()
        .receipt
        .ok_or("replay receipt missing")?;
    assert_eq!(receipt.command_id, replay.command_id);

    let conflict = projects
        .create_workspace(request(
            CreateWorkspaceRequest {
                idempotency_key: workspace_key,
                display_name: "different payload".to_owned(),
            },
            &nonce,
        ))
        .await
        .err()
        .ok_or("changed idempotent payload unexpectedly succeeded")?;
    assert_eq!(conflict.code(), Code::AlreadyExists);
    assert!(!conflict.details().is_empty());
    assert!(conflict.metadata().get("x-request-id").is_some());

    let project = projects
        .create_project(request(
            CreateProjectRequest {
                idempotency_key: format!("grpc-project-{nonce}"),
                display_name: "gRPC project".to_owned(),
                workspace_id: workspace.id.clone(),
                goal: "exercise the private workflow contract".to_owned(),
            },
            &nonce,
        ))
        .await?
        .into_inner()
        .project
        .ok_or("project missing")?;
    exercise_conversation(&mut conversations, &nonce, &workspace.id, &project.id).await
}

async fn exercise_conversation(
    conversations: &mut ProjectConversationServiceClient<Channel>,
    nonce: &str,
    workspace_id: &str,
    project_id: &str,
) -> TestResult {
    let conversation = conversations
        .create_conversation(request(
            CreateConversationRequest {
                idempotency_key: format!("grpc-conversation-{nonce}"),
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                title: "gRPC conversation".to_owned(),
            },
            nonce,
        ))
        .await?
        .into_inner()
        .conversation
        .ok_or("conversation missing")?;
    conversations
        .send_message(request(
            SendMessageRequest {
                idempotency_key: format!("grpc-message-{nonce}"),
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation.id.clone(),
                content: "durable admission".to_owned(),
            },
            nonce,
        ))
        .await?;

    let snapshot = conversations
        .get_conversation_snapshot(request(
            GetConversationSnapshotRequest {
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation.id.clone(),
            },
            nonce,
        ))
        .await?
        .into_inner()
        .snapshot
        .ok_or("snapshot missing")?;
    assert_eq!(snapshot.recent_messages.len(), 1);
    assert_eq!(snapshot.nonterminal_turns.len(), 1);

    let history = conversations
        .list_conversation_history(request(
            ListConversationHistoryRequest {
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation.id.clone(),
                snapshot_id: snapshot.history_snapshot_id,
                before_sequence: 0,
                limit: 10,
            },
            nonce,
        ))
        .await?
        .into_inner();
    assert_eq!(history.records.len(), 1);

    exercise_watch(
        conversations,
        nonce,
        workspace_id,
        project_id,
        &conversation.id,
        snapshot.conversation_revision,
    )
    .await
}

async fn exercise_watch(
    conversations: &mut ProjectConversationServiceClient<Channel>,
    nonce: &str,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
    expected_revision: u64,
) -> TestResult {
    let mut watch = conversations
        .watch_conversation(request(
            WatchConversationRequest {
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation_id.to_owned(),
                after_cursor: None,
            },
            nonce,
        ))
        .await?
        .into_inner();
    let first = tokio::time::timeout(Duration::from_secs(3), watch.message())
        .await??
        .ok_or("watch missing snapshot")?;
    let Some(ayni_orchestrator_api::v1::watch_conversation_response::Frame::Snapshot(
        watch_snapshot,
    )) = first.frame
    else {
        return Err("watch did not start at an atomic snapshot".into());
    };
    assert_eq!(watch_snapshot.conversation_revision, expected_revision);
    let response = conversations
        .send_message(request(
            SendMessageRequest {
                idempotency_key: format!("grpc-watch-message-{nonce}"),
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation_id.to_owned(),
                content: "observed admission".into(),
            },
            nonce,
        ))
        .await?
        .into_inner()
        .receipt
        .ok_or("receipt missing")?;
    let update = tokio::time::timeout(Duration::from_secs(3), watch.message())
        .await??
        .ok_or("watch missing durable update")?;
    let Some(ayni_orchestrator_api::v1::watch_conversation_response::Frame::Update(update)) =
        update.frame
    else {
        return Err("watch did not deliver admitted update".into());
    };
    assert_eq!(update.cursor, response.resume_cursor);
    assert_eq!(update.conversation_revision, response.conversation_revision);
    assert_eq!(
        update.kind,
        ayni_orchestrator_api::v1::ConversationUpdateKind::MessageAdmitted as i32
    );
    drop(watch);
    let cross_scope = conversations
        .watch_conversation(request(
            WatchConversationRequest {
                workspace_id: workspace_id.to_owned(),
                project_id: project_id.to_owned(),
                conversation_id: conversation_id.to_owned(),
                after_cursor: Some("workflow-cursor-v1:other:1".into()),
            },
            nonce,
        ))
        .await
        .err()
        .ok_or("cross-scope watch succeeded")?;
    assert_eq!(cross_scope.code(), Code::InvalidArgument);
    Ok(())
}

fn request<T>(message: T, request_id: &str) -> Request<T> {
    let mut request = Request::new(message);
    if let Ok(value) = request_id.parse() {
        request.metadata_mut().insert("x-request-id", value);
    }
    request
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL gate"]
async fn plan_acceptance_client_recovers_exact_evidence_after_listener_restart() -> TestResult {
    let database_url = env::var(TEST_URL_VARIABLE)?;
    let (address, shutdown, server) = start_server(&database_url)?;
    connect(address).await?;
    let retained = plan_acceptance::run(format!("http://{address}"), None)
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
    shutdown.send(true)?;
    server.await??;
    let (address, shutdown, server) = start_server(&database_url)?;
    connect(address).await?;
    let verified = plan_acceptance::run(format!("http://{address}"), Some(retained))
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
    let result: serde_json::Value = serde_json::from_slice(&verified)?;
    assert_eq!(result["plan_admission_recovery"], "passed");
    shutdown.send(true)?;
    server.await??;
    Ok(())
}
