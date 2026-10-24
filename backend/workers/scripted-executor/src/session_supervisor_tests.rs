#![allow(
    clippy::unwrap_used,
    reason = "fixture construction and assertions fail immediately on unexpected state"
)]
use super::*;
use crate::session_worker::provider_session::Preparation;
use axum::{Router, http::StatusCode, response::IntoResponse as _, routing::post};
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        AgentExecutionConfiguration, ConversationContextManifest, ConversationTurnBinding,
        ConversationTurnRequested,
    },
    workflow::{APPLICATION_SOURCE, WorkflowEventMetadata, encode_command},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn command() -> Vec<u8> {
    let turn = uuid::Uuid::now_v7().to_string();
    let id = uuid::Uuid::now_v7().to_string();
    let payload = ConversationTurnRequested {
        binding: Some(ConversationTurnBinding {
            workspace_id: uuid::Uuid::now_v7().to_string(),
            project_id: uuid::Uuid::now_v7().to_string(),
            conversation_id: uuid::Uuid::now_v7().to_string(),
            turn_id: turn.clone(),
            assignment_id: uuid::Uuid::now_v7().to_string(),
            executor_generation: 1,
            execution_binding_revision: "openai-responses-v1".into(),
            context_revision: 1,
            command_event_id: id.clone(),
            command_digest: String::new(),
        }),
        input_message_id: uuid::Uuid::now_v7().to_string(),
        input_content: "hello".into(),
        context: Vec::new(),
        session: Some(SessionExecutionScope {
            session_id: uuid::Uuid::now_v7().to_string(),
            generation: 1,
            idle_timeout_seconds: 30,
        }),
        frozen_configuration: Some(AgentExecutionConfiguration {
            revision_id: uuid::Uuid::now_v7().to_string(),
            instructions: "frozen".into(),
            provider: "openai".into(),
            model: "fixture-model".into(),
            max_input_tokens: 2048,
            max_output_tokens: 512,
            max_total_tokens: 2560,
            idle_timeout_seconds: 30,
        }),
        context_manifest: Some(ConversationContextManifest {
            id: turn,
            digest: format!("sha256:{}", "a".repeat(64)),
            record_count: 1,
            inline_complete: false,
        }),
        project_status: None,
    };
    encode_command(
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
    .unwrap()
}

async fn disabled_profile(mode: &'static str) -> (ExecutionResult, usize, usize) {
    let exact = command();
    let (identity, _, _) = decode_command(&exact).unwrap();
    let expected = ExecutionResult {
        command_id: identity.id.clone(),
        command_digest: digest(&exact),
        response_content: if mode == "retained" {
            "retained paid response".into()
        } else {
            String::new()
        },
        failure_reason: if mode == "sending" {
            "provider_uncertain".into()
        } else {
            String::new()
        },
        provider_call: None,
        plan_proposal: None,
    };
    let admissions = Arc::new(AtomicUsize::new(0));
    let sends = Arc::new(AtomicUsize::new(0));
    let admit_count = admissions.clone();
    let send_count = sends.clone();
    let retained = expected.clone();
    let sent = expected;
    let app=Router::new()
        .route("/authority",post(move |headers:axum::http::HeaderMap,axum::Json(body):axum::Json<serde_json::Value>| {
            let count=admit_count.clone();async move {
                assert_eq!(headers["authorization"],"Bearer fixture-authority-token");
                if body["action"]=="admit" {count.fetch_add(1,Ordering::SeqCst);}
                axum::Json(json!({"records":[],"next_after_ordinal":0,"allowed":true,"cancellation_requested":false,"state":if mode=="sending"{"sending"}else{"processing"}}))
            }
        }))
        .route("/v1/provider-prepare",post(move ||{let result=retained.clone();async move {
            if mode=="fresh" {return StatusCode::FAILED_DEPENDENCY.into_response();}
            axum::Json(Preparation {call_id:result.command_id.clone(),request_digest:format!("sha256:{}","b".repeat(64)),context_digest:format!("sha256:{}","a".repeat(64)),reserved_input_tokens:1200,reserved_output_tokens:512,reserved_total_tokens:1712,state:if mode=="retained"{"completed"}else{"prepared"}.into(),result:if mode=="retained"{Some(result)}else{None}}).into_response()
        }}))
        .route("/v1/provider-send",post(move |axum::Json(body):axum::Json<serde_json::Value>|{let result=sent.clone();let count=send_count.clone();async move {assert_eq!(body["allowed"],false);count.fetch_add(1,Ordering::SeqCst);axum::Json(result)}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let profile = ProviderRuntime {
        enabled: false,
        authority_url: format!("{base}/authority"),
        authority_token: "fixture-authority-token".into(),
    };
    let result = execute_provider_with_runtime(
        &reqwest::Client::new(),
        &base,
        "fixture-session-token",
        &exact,
        &profile,
    )
    .await
    .unwrap();
    server.abort();
    (
        result,
        admissions.load(Ordering::SeqCst),
        sends.load(Ordering::SeqCst),
    )
}

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn disabled_supervisor_profile_replays_paid_result_without_new_admission_requires_loopback() {
    let (result, admitted, sent) = disabled_profile("retained").await;
    assert_eq!(result.response_content, "retained paid response");
    assert_eq!(admitted, 0);
    assert_eq!(sent, 0);
}

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn disabled_supervisor_profile_reports_missing_credentials_for_fresh_work_requires_loopback()
{
    let (result, admitted, sent) = disabled_profile("fresh").await;
    assert_eq!(result.failure_reason, "provider_credentials_unavailable");
    assert_eq!(admitted, 0);
    assert_eq!(sent, 0);
}

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn disabled_supervisor_profile_recovers_sending_as_uncertain_without_admission_requires_loopback()
 {
    let (result, admitted, sent) = disabled_profile("sending").await;
    assert_eq!(result.failure_reason, "provider_uncertain");
    assert_eq!(admitted, 0);
    assert_eq!(sent, 1);
}

#[test]
fn changed_worker_recipes_adopt_the_same_generation_and_retained_volume() {
    let (_, _, command) = decode_command(&command()).unwrap();
    let scope = command.session.unwrap();
    let binding = command.binding.unwrap();
    let name = name(&scope).unwrap();
    let scripted = desired_pod(&name, &scope, &binding, "worker:old", None).unwrap();
    let upgraded = desired_pod(&name, &scope, &binding, "worker:new", None).unwrap();
    assert!(!same_recipe(&scripted, &upgraded));
    let enabled = desired_pod(
        &name,
        &scope,
        &binding,
        "worker:new",
        Some("https://api.openai.com/v1"),
    )
    .unwrap();
    assert!(!same_recipe(&upgraded, &enabled));
    let changed_endpoint = desired_pod(
        &name,
        &scope,
        &binding,
        "worker:new",
        Some("http://fixture:8080/v1"),
    )
    .unwrap();
    assert!(!same_recipe(&enabled, &changed_endpoint));
    assert!(same_recipe(&enabled, &enabled));
    let mut old_contract = scripted;
    old_contract.metadata.annotations = None;
    assert!(!same_recipe(&old_contract, &upgraded));
    for pod in [upgraded, enabled, changed_endpoint] {
        assert_eq!(pod.metadata.name.as_deref(), Some(name.as_str()));
        let volumes = pod.spec.unwrap().volumes.unwrap();
        let retained = volumes
            .iter()
            .find(|volume| volume.name == "ledger")
            .unwrap()
            .persistent_volume_claim
            .as_ref()
            .unwrap();
        assert_eq!(retained.claim_name, name);
    }
    let mut changed_idle = scope.clone();
    changed_idle.idle_timeout_seconds = 60;
    let idle = desired_pod(&name, &changed_idle, &binding, "worker:new", None).unwrap();
    let baseline = desired_pod(&name, &scope, &binding, "worker:new", None).unwrap();
    assert!(same_recipe(&idle, &baseline));
}
