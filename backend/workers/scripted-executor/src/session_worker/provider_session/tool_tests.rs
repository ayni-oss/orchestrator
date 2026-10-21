use super::*;

async fn tool_fixture() -> Fixture {
    let count = Arc::new(AtomicUsize::new(0));
    let inner = count.clone();
    let arrived = Arc::new(tokio::sync::Notify::new());
    let notify = arrived.clone();
    let app=Router::new().route("/v1/responses",post(move |axum::Json(request):axum::Json<serde_json::Value>| {
        let count=inner.clone();let notify=notify.clone();async move {
            let round=count.fetch_add(1,Ordering::SeqCst);notify.notify_one();
            if round==0 {
                serde_json::json!({"id":"resp_tools","model":"fixture-model","status":"completed","output":[
                    {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Reading scoped inputs."}]},
                    {"type":"function_call","call_id":"call_instructions","name":"read_workspace_instructions","arguments":"{}"},
                    {"type":"function_call","call_id":"call_project","name":"read_project_status","arguments":"{}"},
                    {"type":"function_call","call_id":"call_history","name":"read_conversation_history","arguments":"{}"}
                ],"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}).to_string()
            } else {
                let items=request["input"].as_array().unwrap();
                assert_eq!(items.iter().filter(|item|item["role"]=="assistant" && item["content"]=="Reading scoped inputs.").count(),1);
                let outputs=items.iter().filter(|item|item["type"]=="function_call_output").map(|item|serde_json::from_str::<serde_json::Value>(item["output"].as_str().unwrap()).unwrap()).collect::<Vec<_>>();
                assert_eq!(outputs.len(),3);
                assert_eq!(outputs[0]["instructions"],"frozen instructions");
                assert_eq!(outputs[1]["objective"],"frozen objective");
                assert_eq!(outputs[2]["records"][0]["content"],"hello");
                SUCCESS.into()
            }
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

fn tool_input(state: &WorkerState) -> PrepareInput {
    let mut input = command(state);
    let (identity, metadata, mut payload) = decode_command(&input.command_bytes).unwrap();
    let config = payload.frozen_configuration.as_mut().unwrap();
    config.max_input_tokens = 65536;
    config.max_output_tokens = 4096;
    config.max_total_tokens = 69632;
    payload.project_status = Some(ayni_orchestrator_events::v1::ProjectStatusSnapshot {
        snapshot_id: identity.id.clone(),
        project_id: state.project_id.clone(),
        objective: "frozen objective".into(),
        status: "registered".into(),
        context_revision: 1,
        project_resource_version: 1,
    });
    input.command_bytes = encode_command(&identity, &metadata, &payload).unwrap();
    input
}

#[tokio::test]
async fn three_scoped_reads_are_retained_and_usage_is_cumulative() {
    let fixture = tool_fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(tool_input(&state)),
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
    let call = result.provider_call.unwrap();
    assert_eq!(call.total_tokens, Some(127));
    assert_eq!(call.input_tokens, Some(104));
    assert_eq!(call.tool_invocations.len(), 3);
    assert!(
        call.tool_invocations
            .iter()
            .all(|tool| tool.state == "completed")
    );
    {
        let connection = state.connection.lock().unwrap();
        let saved: String = connection
            .query_row("SELECT state FROM provider_loops", [], |row| row.get(0))
            .unwrap();
        drop(connection);
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["exchanges"].as_array().unwrap().len(), 2);
        assert_eq!(
            saved["tools"]["call_project"]["output"]
                .as_str()
                .map(|output| output.contains("frozen objective")),
            Some(true)
        );
    }
    let replay = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, false, "uncertain")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(replay.response_content, "bounded answer");
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retained_final_exchange_recovers_without_resending_after_restart() {
    let fixture = tool_fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let original = worker(&path, &fixture.url, None);
    let prepared = prepare(
        State(original.clone()),
        auth(&original),
        axum::Json(tool_input(&original)),
    )
    .await
    .unwrap()
    .0;
    let _ = send(
        State(original.clone()),
        auth(&original),
        axum::Json(sending(&prepared, true, "sending")),
    )
    .await
    .unwrap();
    // Interrupt after the normalized final exchange commit, before terminal-report retention.
    original
        .connection
        .lock()
        .unwrap()
        .execute_batch("DELETE FROM results; UPDATE provider_calls SET state='sending';")
        .unwrap();
    let restarted = worker(&path, &fixture.url, Some(&original));
    let recovered = send(
        State(restarted.clone()),
        auth(&restarted),
        axum::Json(sending(&prepared, false, "uncertain")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(recovered.response_content, "bounded answer");
    assert_eq!(recovered.provider_call.unwrap().total_tokens, Some(127));
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unresolved_continuation_keeps_the_entire_turn_usage_unknown() {
    let fixture = tool_fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(tool_input(&state)),
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
    {
        let connection = state.connection.lock().unwrap();
        let json: String = connection
            .query_row("SELECT state FROM provider_loops", [], |row| row.get(0))
            .unwrap();
        let mut checkpoint: serde_json::Value = serde_json::from_str(&json).unwrap();
        checkpoint["phase"] = "sending".into();
        connection
            .execute(
                "UPDATE provider_loops SET state=?1",
                [checkpoint.to_string()],
            )
            .unwrap();
        connection
            .execute_batch(
                "DELETE FROM results; UPDATE provider_calls SET state='sending',settled=0;",
            )
            .unwrap();
    }
    let recovered = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, false, "uncertain")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(recovered.failure_reason, "provider_uncertain");
    let evidence = recovered.provider_call.unwrap();
    assert_eq!(evidence.total_tokens, None);
    assert_eq!(evidence.tool_invocations.len(), 3);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
    assert!(!idle_and_resolved(&state));
}

#[tokio::test]
async fn ledger_cleanup_requires_matching_decision_and_known_usage() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let known = prepare(
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
        axum::Json(sending(&known, true, "sending")),
    )
    .await
    .unwrap();
    // Neither elapsed retention nor a terminal provider response is an application decision.
    let pending = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(command(&state)),
    )
    .await
    .unwrap()
    .0;
    assert!(
        state
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM provider_loops WHERE command_id=?1)",
                [&known.call_id],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    super::super::acknowledge(State(state.clone()), auth(&state), known.call_id.clone())
        .await
        .unwrap();
    let released: bool = state
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT request IS NULL AND settled=1 FROM provider_calls WHERE command_id=?1",
            [&known.call_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(released);
    let _ = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&pending, true, "sending")),
    )
    .await
    .unwrap();
    super::super::acknowledge(State(state.clone()), auth(&state), pending.call_id.clone())
        .await
        .unwrap();
    // Unknown usage remains charged even after the exact application decision.
    state
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE provider_calls SET settled=0 WHERE command_id=?1",
            [&pending.call_id],
        )
        .unwrap();
    state
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE results SET acknowledged_at=0", [])
        .unwrap();
    let _ = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(command(&state)),
    )
    .await
    .unwrap();
    assert!(!retains(&state, "provider_loops", &known.call_id));
    assert!(retains(&state, "provider_loops", &pending.call_id));
    assert!(retains(&state, "results", &pending.call_id));
}

#[tokio::test]
async fn retained_tool_result_does_not_authorize_another_read_after_restart() {
    let fixture = tool_fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(tool_input(&state)),
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
    {
        let connection = state.connection.lock().unwrap();
        let json: String = connection
            .query_row("SELECT state FROM provider_loops", [], |row| row.get(0))
            .unwrap();
        let mut checkpoint: serde_json::Value = serde_json::from_str(&json).unwrap();
        checkpoint["phase"] = "result".into();
        checkpoint["round"] = 0.into();
        checkpoint["tools"] = serde_json::json!({});
        checkpoint["response"] = checkpoint["exchanges"][0]["response"].clone();
        checkpoint["usage"] =
            serde_json::json!({"input_tokens":100,"output_tokens":20,"total_tokens":120});
        connection
            .execute(
                "UPDATE provider_loops SET state=?1",
                [checkpoint.to_string()],
            )
            .unwrap();
        connection
            .execute_batch(
                "DELETE FROM results; UPDATE provider_calls SET state='sending',settled=0;",
            )
            .unwrap();
    }
    let recovered = send(
        State(state.clone()),
        auth(&state),
        axum::Json(sending(&prepared, false, "uncertain")),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(recovered.failure_reason, "provider_interrupted");
    let evidence = recovered.provider_call.unwrap();
    assert_eq!(evidence.total_tokens, Some(120));
    assert!(evidence.tool_invocations.is_empty());
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
}

fn retains(state: &WorkerState, table: &str, command_id: &str) -> bool {
    state
        .connection
        .lock()
        .unwrap()
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE command_id=?1)"),
            [command_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn unknown_usage_blocks_tools_and_paid_continuations() {
    let fixture=fixture(r#"{"id":"resp_unknown_tools","model":"fixture-model","status":"completed","output":[{"type":"function_call","call_id":"call_instructions","name":"read_workspace_instructions","arguments":"{}"}]}"#,Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(tool_input(&state)),
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
    assert_eq!(result.failure_reason, "provider_usage_unknown");
    let evidence = result.provider_call.unwrap();
    assert_eq!(evidence.total_tokens, None);
    assert!(evidence.tool_invocations.is_empty());
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    super::super::super::acknowledge(State(state.clone()), auth(&state), prepared.call_id)
        .await
        .unwrap();
    assert!(!idle_and_resolved(&state));
}

#[tokio::test]
async fn a_failed_result_checkpoint_preserves_sending_uncertainty_without_retry() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
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
    state.connection.lock().unwrap().execute_batch("CREATE TRIGGER refuse_result BEFORE UPDATE ON provider_loops WHEN json_extract(NEW.state,'$.phase')='result' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    for _ in 0..2 {
        let result = send(
            State(state.clone()),
            auth(&state),
            axum::Json(sending(&prepared, true, "sending")),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(result.failure_reason, "provider_uncertain");
        assert_eq!(result.provider_call.unwrap().total_tokens, None);
    }
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    let phase: String = state
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT json_extract(state,'$.phase') FROM provider_loops",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(phase, "sending");
}

#[tokio::test]
async fn preparation_replays_exact_retained_request_even_when_new_catalog_exceeds_its_budget() {
    let fixture = fixture(SUCCESS, Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let mut input = command(&state);
    let (identity, metadata, mut payload) = decode_command(&input.command_bytes).unwrap();
    payload
        .frozen_configuration
        .as_mut()
        .unwrap()
        .max_input_tokens = 1;
    input.command_bytes = encode_command(&identity, &metadata, &payload).unwrap();
    let command_digest = digest(&input.command_bytes);
    let request = b"retained original request";
    let request_digest = digest(request);
    let evidence = ProviderCallEvidence {
        call_id: identity.id.clone(),
        reserved_input_tokens: 1,
        reserved_output_tokens: 512,
        reserved_total_tokens: 513,
        request_digest: request_digest.clone(),
        ..Default::default()
    };
    state.connection.lock().unwrap().execute("INSERT INTO provider_calls(command_id,command_digest,request,request_digest,evidence,state) VALUES(?1,?2,?3,?4,?5,'prepared')",params![identity.id,command_digest,request,request_digest,serde_json::to_string(&evidence).unwrap()]).unwrap();
    let replay = prepare(State(state.clone()), auth(&state), axum::Json(input))
        .await
        .unwrap()
        .0;
    assert_eq!(replay.request_digest, request_digest);
    assert_eq!(replay.reserved_input_tokens, 1);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn reasoning_with_tool_calls_is_rejected_without_losing_usage_or_running_tools() {
    let fixture=fixture(r#"{"id":"resp_reasoning_tools","model":"fixture-model","status":"completed","output":[{"type":"reasoning","summary":[{"text":"private reasoning"}]},{"type":"function_call","call_id":"call_instructions","name":"read_workspace_instructions","arguments":"{}"}],"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}"#,Duration::ZERO).await;
    let directory = tempfile::tempdir().unwrap();
    let state = worker(&directory.path().join("ledger.sqlite"), &fixture.url, None);
    let prepared = prepare(
        State(state.clone()),
        auth(&state),
        axum::Json(tool_input(&state)),
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
    assert_eq!(result.failure_reason, "provider_tool_rejected");
    let evidence = result.provider_call.unwrap();
    assert_eq!(evidence.total_tokens, Some(120));
    assert!(evidence.tool_invocations.is_empty());
    let retained: String = state
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT state FROM provider_loops", [], |row| row.get(0))
        .unwrap();
    assert!(!retained.contains("private reasoning"));
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
}
