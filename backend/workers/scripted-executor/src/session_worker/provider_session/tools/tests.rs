use super::*;
fn execution() -> ProviderLoop {
    ProviderLoop {
        request: json!({}),
        phase: "ready".into(),
        round: 0,
        response: None,
        usage: TokenUsage::default(),
        sources: BTreeMap::from([(
            "read_workspace_instructions".into(),
            json!({"instructions":"frozen"}),
        )]),
        tools: BTreeMap::new(),
        exchanges: Vec::new(),
        mcp_tools: BTreeMap::new(),
        trace_context: None,
        turn_id: String::new(),
    }
}
fn call(id: &str, args: &str) -> FunctionCall {
    FunctionCall {
        call_id: id.into(),
        name: "read_workspace_instructions".into(),
        arguments: args.into(),
    }
}
#[test]
fn scoped_invocation_replays_exact_output_and_rejects_changed_identity_or_scope() {
    let mut execution = execution();
    let request = call("same", "{}");
    let original = invoke(&mut execution, &request).unwrap();
    execution
        .sources
        .insert(request.name.clone(), json!({"instructions":"changed"}));
    assert_eq!(invoke(&mut execution, &request).unwrap(), original);
    assert_eq!(
        invoke(
            &mut execution,
            &call("same", "{\"conversation_id\":\"foreign\"}")
        ),
        Err("provider_tool_rejected")
    );
    assert_eq!(
        invoke(
            &mut execution,
            &call("foreign", "{\"project_id\":\"foreign\"}")
        ),
        Err("provider_tool_rejected")
    );
    assert_eq!(execution.tools["foreign"].state, "rejected");
}
#[test]
fn invocation_count_argument_size_and_output_size_are_hard_bounds() {
    let mut execution = execution();
    for index in 0..8 {
        invoke(&mut execution, &call(&format!("call_{index}"), "{}")).unwrap();
    }
    assert_eq!(
        invoke(&mut execution, &call("ninth", "{}")),
        Err("provider_tool_limit")
    );
    let mut execution = super::tests::execution();
    assert_eq!(
        invoke(&mut execution, &call("large", &" ".repeat(4097))),
        Err("provider_tool_rejected")
    );
    execution.sources.insert(
        "read_workspace_instructions".into(),
        json!({"instructions":"x".repeat(32768)}),
    );
    assert_eq!(
        invoke(&mut execution, &call("output", "{}")),
        Err("provider_tool_output_limit")
    );
    assert!(execution.tools["output"].output.is_empty());
    let mut unknown = call("shell", "{}");
    unknown.name = "shell".into();
    assert_eq!(
        invoke(&mut execution, &unknown),
        Err("provider_tool_rejected")
    );
}
#[tokio::test]
async fn ambiguous_mcp_invocation_replay_cannot_send_again() {
    let mut execution = execution();
    let call = FunctionCall {
        call_id: "mcp_same".into(),
        name: "mcp_lookup".into(),
        arguments: "{}".into(),
    };
    execution.tools.insert(
        call.call_id.clone(),
        RetainedTool {
            call: call.clone(),
            input_digest: digest(b"{}"),
            output: String::new(),
            state: "sending".into(),
            consent_id: None,
        },
    );
    let state = WorkerState {
        active: Mutex::new(std::collections::HashSet::new()),
        provider: Mutex::new(None),
        session_id: "session".into(),
        generation: 1,
        conversation_id: "conversation".into(),
        workspace_id: "workspace".into(),
        project_id: "project".into(),
        token: "token".into(),
        connection: Mutex::new(Connection::open_in_memory().unwrap()),
        last_request: Mutex::new(Instant::now()),
        idle_timeout: Mutex::new(Duration::from_secs(30)),
    };
    let input = SendInput {
        command_id: "command".into(),
        request_digest: "digest".into(),
        allowed: true,
        state: "sending".into(),
    };
    assert_eq!(
        invoke_mcp(&state, &input, &mut execution, &call).await,
        Err("provider_tool_uncertain")
    );
    let mut changed = call.clone();
    changed.arguments = "{\"changed\":true}".into();
    assert_eq!(
        invoke_mcp(&state, &input, &mut execution, &changed).await,
        Err("provider_tool_rejected")
    );
}
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one retained-ledger external-effect scenario"
)]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn exact_external_effect_consent_is_consumed_once_and_survives_ledger_reopen_requires_loopback()
 {
    use axum::{Json, Router, response::IntoResponse, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let app = Router::new().route("/mcp", post(move |Json(body): Json<Value>| {
            let count = observed.clone();
            async move { match body["method"].as_str() {
                Some("initialize") => Json(json!({"jsonrpc":"2.0","id":"initialize","result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}}}})).into_response(),
                Some("notifications/initialized") => StatusCode::ACCEPTED.into_response(),
                Some("tools/list") => Json(json!({"jsonrpc":"2.0","id":"tools","result":{"tools":(["create","create500","large"].iter().map(|name|json!({"name":name,"inputSchema":{"type":"object","properties":{},"required":[],"additionalProperties":false}})).collect::<Vec<_>>())}})).into_response(),
                Some("tools/call") => { count.fetch_add(1, Ordering::SeqCst); if body["params"]["name"] == "create500" {return StatusCode::INTERNAL_SERVER_ERROR.into_response();} else if body["params"]["name"] == "large" {return Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":"x".repeat(5000)}})).into_response();} Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":[{"type":"text","text":"created"}]}})).into_response() },
                _ => StatusCode::BAD_REQUEST.into_response(),
            }}
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let command_id = uuid::Uuid::now_v7().to_string();
    let state = WorkerState {
        active: Mutex::new(std::collections::HashSet::new()),
        provider: Mutex::new(None),
        session_id: "session".into(),
        generation: 1,
        conversation_id: "conversation".into(),
        workspace_id: "workspace".into(),
        project_id: "project".into(),
        token: "token".into(),
        connection: Mutex::new(Connection::open_in_memory().unwrap()),
        last_request: Mutex::new(Instant::now()),
        idle_timeout: Mutex::new(Duration::from_secs(30)),
    };
    state
        .connection
        .lock()
        .unwrap()
        .execute_batch(super::super::super::SCHEMA)
        .unwrap();
    state
        .connection
        .lock()
        .unwrap()
        .execute_batch(super::super::SCHEMA)
        .unwrap();
    state.connection.lock().unwrap().execute("INSERT INTO provider_calls(command_id,command_digest,request_digest,evidence,state) VALUES(?1,'digest','digest','{}','sending')",[&command_id]).unwrap();
    let mut execution = execution();
    execution.mcp_tools.insert("mcp_create".into(), super::super::mcp::Tool {
            alias:"mcp_create".into(),name:"create".into(),endpoint,description:"Create exact reviewed item".into(),input_schema:json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),read_only:false,max_output_bytes:4096,timeout_seconds:5,bearer_file:None,
            consent:Some(super::super::mcp::Consent {command_id:command_id.clone(),arguments_digest:digest(b"{}"),acknowledge_external_effect:true}),
        });
    state
        .connection
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO provider_loops(command_id,state) VALUES(?1,?2)",
            params![command_id, serde_json::to_string(&execution).unwrap()],
        )
        .unwrap();
    let input = SendInput {
        command_id: command_id.clone(),
        request_digest: "digest".into(),
        allowed: true,
        state: "sending".into(),
    };
    let call = FunctionCall {
        call_id: "effect_1".into(),
        name: "mcp_create".into(),
        arguments: "{}".into(),
    };
    let result = invoke_mcp(&state, &input, &mut execution, &call)
        .await
        .unwrap();
    assert!(result.contains("created"));
    let json: String = state
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM provider_loops WHERE command_id=?1",
            [&command_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut reopened: ProviderLoop = serde_json::from_str(&json).unwrap();
    assert_eq!(
        invoke_mcp(&state, &input, &mut reopened, &call)
            .await
            .unwrap(),
        result
    );
    let second = FunctionCall {
        call_id: "effect_2".into(),
        ..call.clone()
    };
    assert_eq!(
        invoke_mcp(&state, &input, &mut reopened, &second).await,
        Err("provider_tool_rejected")
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    for (index, name) in ["create500", "large"].iter().enumerate() {
        let mut tool = reopened.mcp_tools["mcp_create"].clone();
        tool.alias = format!("mcp_failure_{index}");
        tool.name = (*name).into();
        let failed = FunctionCall {
            call_id: format!("failed_{index}"),
            name: tool.alias.clone(),
            arguments: "{}".into(),
        };
        reopened.mcp_tools.insert(tool.alias.clone(), tool);
        assert_eq!(
            invoke_mcp(&state, &input, &mut reopened, &failed).await,
            Err("provider_tool_uncertain")
        );
        assert_eq!(reopened.tools[&failed.call_id].state, "uncertain");
        assert_eq!(
            invoke_mcp(&state, &input, &mut reopened, &failed).await,
            Err("provider_tool_uncertain")
        );
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        3,
        "post-commit errors cannot repeat effects"
    );
    server.abort();
}
#[test]
fn retained_quota_counts_utf8_bytes_for_loop_results_and_provider_evidence() {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE provider_calls(request BLOB,evidence TEXT);CREATE TABLE results(result TEXT);CREATE TABLE provider_loops(command_id TEXT,state TEXT);").unwrap();
    let text = serde_json::to_string(&json!({"content":"é世界🦀".repeat(10)})).unwrap();
    connection
        .execute(
            "INSERT INTO provider_calls VALUES(?1,?2)",
            params![text.as_bytes(), text],
        )
        .unwrap();
    connection
        .execute("INSERT INTO results VALUES(?1)", [&text])
        .unwrap();
    connection
        .execute(
            "INSERT INTO provider_loops VALUES('retained',?1),('active',?1)",
            [&text],
        )
        .unwrap();
    assert_eq!(
        retained_bytes(&connection, "active").unwrap(),
        i64::try_from(text.len() * 4).unwrap()
    );
}
