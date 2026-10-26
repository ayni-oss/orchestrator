use super::*;
use serde_json::{Value, json};

fn arguments() -> Value {
    json!({"content":{"title":"Proposed work","objective":"Define next steps","scope":"This project","out_of_scope":"Execution","constraints":["Human review required"],"unresolved_questions":["Select a source revision"],"tasks":[]},"assumptions":["Repository is unverified"],"evidence":["User supplied the objective"]})
}

async fn proposal_fixture(args: Value, mixed: bool) -> Fixture {
    let count = Arc::new(AtomicUsize::new(0));
    let arrived = Arc::new(tokio::sync::Notify::new());
    let inner = count.clone();
    let notify = arrived.clone();
    let app = Router::new().route("/v1/responses", post(move || {
        let inner = inner.clone(); let notify = notify.clone(); let args=args.clone();
        async move {
            inner.fetch_add(1,Ordering::SeqCst); notify.notify_one();
            let mut output=vec![json!({"type":"function_call","name":"propose_plan","call_id":"proposal_invocation","arguments":args.to_string()})];
            if mixed { output.push(json!({"type":"function_call","name":"read_project_status","call_id":"mixed_read","arguments":"{}"})); }
            json!({"id":"resp_proposal","model":"fixture-model","status":"completed","output":output,"usage":{"input_tokens":4,"output_tokens":3,"total_tokens":7}}).to_string()
        }
    }));
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

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn retained_proposal_recovers_exact_invocation_and_bytes_without_paid_replay_requires_loopback()
 {
    let mut args = arguments();
    // Near the content limit, exercising JSON escaping and the session HTTP/result bounds.
    args["content"]["constraints"] = json!(vec!["\\\"".repeat(500); 16]);
    let expected = args.to_string().into_bytes();
    let fixture = proposal_fixture(args, false).await;
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
    let accepted = send(
        State(original.clone()),
        auth(&original),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap()
    .0;
    assert!(accepted.failure_reason.is_empty());
    let proposal = accepted.plan_proposal.unwrap();
    assert_eq!(proposal.invocation_id, "proposal_invocation");
    assert_eq!(proposal.proposal_json, expected);
    assert!(serde_json::to_vec(&proposal).unwrap().len() > 40 * 1024);
    original
        .connection
        .lock()
        .unwrap()
        .execute_batch("DELETE FROM results; UPDATE provider_calls SET state='sending';")
        .unwrap();
    let restarted = worker(&path, &fixture.url, Some(&original));
    let replay = send(
        State(restarted.clone()),
        auth(&restarted),
        axum::Json(sending(&prepared, false, "uncertain")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(replay.plan_proposal.unwrap(), proposal);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn malformed_mixed_and_scope_selecting_proposals_fail_without_plan_output_requires_loopback()
{
    let mut foreign = arguments();
    foreign["conversation_id"] = json!("foreign");
    for (args, mixed) in [(json!({}), false), (foreign, false), (arguments(), true)] {
        let fixture = proposal_fixture(args, mixed).await;
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
        assert_eq!(result.failure_reason, "invalid_plan_proposal");
        assert!(result.plan_proposal.is_none());
        assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn cancellation_before_checkpoint_recovery_cannot_emit_retained_proposal_requires_loopback() {
    let fixture = proposal_fixture(arguments(), false).await;
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
    let _ = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap();
    state
        .connection
        .lock()
        .unwrap()
        .execute_batch(
            "DELETE FROM results; UPDATE provider_calls SET state='sending',cancelled=1;",
        )
        .unwrap();
    let result = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, false, "cancelled")),
    )
    .await
    .unwrap()
    .0;
    assert!(result.plan_proposal.is_none());
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}
