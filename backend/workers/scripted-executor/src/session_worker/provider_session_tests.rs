#![allow(
    clippy::unwrap_used,
    reason = "fixture construction and assertions fail immediately on unexpected state"
)]

use super::*;
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        AgentExecutionConfiguration, ConversationContextManifest, ConversationTurnBinding,
        ConversationTurnRequested, SessionExecutionScope,
    },
    workflow::{APPLICATION_SOURCE, WorkflowEventMetadata, encode_command},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    url: String,
    count: Arc<AtomicUsize>,
    arrived: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture(body: &'static str, delay: Duration) -> Fixture {
    let count = Arc::new(AtomicUsize::new(0));
    let arrived = Arc::new(tokio::sync::Notify::new());
    let inner = count.clone();
    let notify = arrived.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(
            move |headers: HeaderMap, axum::Json(request): axum::Json<serde_json::Value>| {
                let count = inner.clone();
                let notify = notify.clone();
                async move {
                    assert!(headers.get("x-client-request-id").is_some());
                    assert_eq!(request["store"], false);
                    assert_eq!(request["truncation"], "disabled");
                    assert_eq!(
                        request["tools"],
                        ayni_orchestrator_providers::responses::scoped_tools()
                    );
                    count.fetch_add(1, Ordering::SeqCst);
                    notify.notify_one();
                    tokio::time::sleep(delay).await;
                    body
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture {
        url,
        count,
        arrived,
        task,
    }
}
const SUCCESS: &str = r#"{"id":"resp_fixture","model":"fixture-model","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"bounded answer"}]}],"usage":{"input_tokens":4,"output_tokens":3,"total_tokens":7}}"#;

fn worker(path: &std::path::Path, endpoint: &str, prior: Option<&WorkerState>) -> Arc<WorkerState> {
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(super::super::SCHEMA).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    Arc::new(WorkerState {
        session_id: prior.map_or_else(
            || uuid::Uuid::now_v7().to_string(),
            |state| state.session_id.clone(),
        ),
        generation: 1,
        conversation_id: prior.map_or_else(
            || uuid::Uuid::now_v7().to_string(),
            |state| state.conversation_id.clone(),
        ),
        workspace_id: prior.map_or_else(
            || uuid::Uuid::now_v7().to_string(),
            |state| state.workspace_id.clone(),
        ),
        project_id: prior.map_or_else(
            || uuid::Uuid::now_v7().to_string(),
            |state| state.project_id.clone(),
        ),
        token: "fixture-session-auth-012345678901234567890123456789".into(),
        connection: Mutex::new(connection),
        last_request: Mutex::new(Instant::now()),
        idle_timeout: Mutex::new(Duration::from_secs(30)),
        active: Mutex::new(std::collections::HashSet::new()),
        provider: Mutex::new(Some(ResponsesClient::new("fixture-key", endpoint).unwrap())),
    })
}
fn auth(state: &WorkerState) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", state.token).parse().unwrap(),
    );
    headers
}
fn command(state: &WorkerState) -> PrepareInput {
    let id = uuid::Uuid::now_v7().to_string();
    let input_id = uuid::Uuid::now_v7().to_string();
    let context = vec![ConversationContextRecord {
        message_id: input_id.clone(),
        sequence: 1,
        role: "user".into(),
        content: "hello".into(),
    }];
    let turn_id = uuid::Uuid::now_v7().to_string();
    let payload = ConversationTurnRequested {
        binding: Some(ConversationTurnBinding {
            workspace_id: state.workspace_id.clone(),
            project_id: state.project_id.clone(),
            conversation_id: state.conversation_id.clone(),
            turn_id: turn_id.clone(),
            assignment_id: uuid::Uuid::now_v7().to_string(),
            executor_generation: 1,
            execution_binding_revision: "openai-responses-v1".into(),
            context_revision: 1,
            command_event_id: id.clone(),
            command_digest: String::new(),
        }),
        input_message_id: input_id,
        input_content: "hello".into(),
        context: context.clone(),
        session: Some(SessionExecutionScope {
            session_id: state.session_id.clone(),
            generation: 1,
            idle_timeout_seconds: 30,
        }),
        frozen_configuration: Some(AgentExecutionConfiguration {
            revision_id: uuid::Uuid::now_v7().to_string(),
            instructions: "frozen instructions".into(),
            provider: "openai".into(),
            model: "fixture-model".into(),
            max_input_tokens: 8192,
            max_output_tokens: 512,
            max_total_tokens: 8704,
            idle_timeout_seconds: 30,
        }),
        context_manifest: Some(ConversationContextManifest {
            id: turn_id,
            digest: context_digest(&context),
            record_count: 1,
            inline_complete: true,
        }),
        project_status: None,
    };
    let command_bytes = encode_command(
        &EventIdentity {
            source: APPLICATION_SOURCE.into(),
            id,
        },
        &WorkflowEventMetadata {
            produced_at: "2026-10-21T12:00:00Z".into(),
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
        },
        &payload,
    )
    .unwrap();
    PrepareInput {
        command_bytes,
        context,
    }
}
fn sending(prepared: &Preparation, allowed: bool, state: &str) -> SendInput {
    SendInput {
        command_id: prepared.call_id.clone(),
        request_digest: prepared.request_digest.clone(),
        allowed,
        state: state.into(),
    }
}

#[tokio::test]
async fn retained_response_replays_across_worker_restart_without_another_paid_call() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let original = worker(&path, &fixture.url, None);
    let input = command(&original);
    let command_bytes = input.command_bytes.clone();
    let context = input.context.clone();
    let prepared = prepare(State(original.clone()), auth(&original), axum::Json(input))
        .await
        .unwrap()
        .0;
    let result = send(
        State(original.clone()),
        auth(&original),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.response_content, "bounded answer");
    assert_eq!(result.provider_call.as_ref().unwrap().total_tokens, Some(7));
    super::super::acknowledge(
        State(original.clone()),
        auth(&original),
        prepared.call_id.clone(),
    )
    .await
    .unwrap();
    let mut restarted = worker(&path, &fixture.url, Some(&original));
    Arc::get_mut(&mut restarted).unwrap().provider = Mutex::new(None);
    drop(original);
    let replay = prepare(
        State(restarted.clone()),
        auth(&restarted),
        axum::Json(PrepareInput {
            command_bytes,
            context,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        replay.result.unwrap().response_content,
        result.response_content
    );
    let replay = send(
        State(restarted.clone()),
        auth(&restarted),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(replay.response_content, result.response_content);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sending_intent_after_restart_is_uncertain_and_never_resent() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let original = worker(&path, &fixture.url, None);
    let prepared = prepare(
        State(original.clone()),
        auth(&original),
        axum::Json(command(&original)),
    )
    .await
    .unwrap()
    .0;
    original
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE provider_calls SET state='sending'", [])
        .unwrap();
    let restarted = worker(&path, &fixture.url, Some(&original));
    drop(original);
    let result = send(
        State(restarted.clone()),
        auth(&restarted),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.failure_reason, "provider_uncertain");
    let evidence = result.provider_call.unwrap();
    assert_eq!(evidence.state, "uncertain");
    assert_eq!(evidence.total_tokens, None);
    assert!(evidence.reserved_total_tokens > 0);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 0);
    assert!(!idle_and_resolved(&restarted));
}

#[tokio::test]
async fn ambiguous_body_is_retained_once_without_a_second_request() {
    let fixture = fixture("{truncated", Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let worker = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(worker.clone()),
        auth(&worker),
        axum::Json(command(&worker)),
    )
    .await
    .unwrap()
    .0;
    for _ in 0..2 {
        let result = send(
            State(worker.clone()),
            auth(&worker),
            axum::Json(sending(&prepared, true, "sending")),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(result.failure_reason, "provider_uncertain");
    }
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_prepared_request_cannot_send() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let worker = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(worker.clone()),
        auth(&worker),
        axum::Json(command(&worker)),
    )
    .await
    .unwrap()
    .0;
    let result = send(
        State(worker.clone()),
        auth(&worker),
        axum::Json(sending(&prepared, false, "cancelled")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.failure_reason, "cancelled");
    assert_eq!(fixture.count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_interrupts_locally_and_keeps_external_outcome_unknown() {
    let fixture = fixture(SUCCESS, Duration::from_secs(60)).await;
    let directory = tempfile::tempdir().unwrap();
    let worker = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let input = command(&worker);
    let digest = digest(&input.command_bytes);
    let prepared = prepare(State(worker.clone()), auth(&worker), axum::Json(input))
        .await
        .unwrap()
        .0;
    let handle = {
        let state = worker.clone();
        let body = sending(&prepared, true, "sending");
        tokio::spawn(
            async move { send(State(state.clone()), auth(&state), axum::Json(body)).await },
        )
    };
    tokio::time::timeout(Duration::from_secs(3), fixture.arrived.notified())
        .await
        .unwrap();
    assert!(matches!(
        send(
            State(worker.clone()),
            auth(&worker),
            axum::Json(sending(&prepared, true, "sending"))
        )
        .await,
        Err(StatusCode::CONFLICT)
    ));
    cancel(
        State(worker.clone()),
        auth(&worker),
        axum::Json(CancelInput {
            command_id: prepared.call_id,
            command_digest: digest,
        }),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(result.failure_reason, "provider_uncertain");
    assert_eq!(result.provider_call.unwrap().total_tokens, None);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unknown_usage_retains_request_and_reservation_after_application_decision() {
    let fixture=fixture(r#"{"id":"resp_unknown","model":"fixture-model","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}]}"#,Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(command(&state)),
    )
    .await
    .unwrap()
    .0;
    let result = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.response_content, "answer");
    assert_eq!(result.provider_call.unwrap().total_tokens, None);
    super::super::acknowledge(State(state.clone()), auth(&state), prepared.call_id)
        .await
        .unwrap();
    *state.last_request.lock().unwrap() = Instant::now()
        .checked_sub(Duration::from_secs(120))
        .unwrap();
    assert!(!idle_and_resolved(&state));
    let retained: bool = state
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT request IS NOT NULL AND settled=0 FROM provider_calls",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(retained);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn output_limit_retains_terminal_usage_and_never_retries() {
    let fixture=fixture(r#"{"id":"resp_incomplete","model":"fixture-model","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[],"usage":{"input_tokens":4,"output_tokens":16,"total_tokens":20}}"#,Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(command(&state)),
    )
    .await
    .unwrap()
    .0;
    for _ in 0..2 {
        let result = send(
            State(state.clone()),
            auth(&state),
            axum::Json(sending(&prepared, true, "sending")),
        )
        .await
        .unwrap()
        .0;
        assert!(result.response_content.is_empty());
        assert_eq!(result.failure_reason, "provider_output_limit");
        let evidence = result.provider_call.unwrap();
        assert_eq!(evidence.state, "failed");
        assert_eq!(evidence.response_id, "resp_incomplete");
        assert_eq!(evidence.total_tokens, Some(20));
    }
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn credentials_appearing_after_worker_start_are_loaded_before_preparation() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    *state.provider.lock().unwrap() = None;
    assert_eq!(
        ensure_provider(&state, || Err(())),
        Err(StatusCode::FAILED_DEPENDENCY)
    );
    ensure_provider(&state, || {
        ResponsesClient::new("fixture-key", &fixture.url).map_err(|_| ())
    })
    .unwrap();
    ensure_provider(&state, || {
        panic!("the admitted request client must remain fixed")
    })
    .unwrap();
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(command(&state)),
    )
    .await
    .unwrap()
    .0;
    let result = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.response_content, "bounded answer");
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}

#[path = "provider_session/tool_tests.rs"]
mod tool_tests;

#[path = "provider_session/proposal_tests.rs"]
mod proposal_tests;
