use super::*;

async fn anthropic_fixture(malformed: bool) -> Fixture {
    let count = Arc::new(AtomicUsize::new(0));
    let inner = count.clone();
    let app=Router::new().route("/v1/messages",post(move |headers:HeaderMap,axum::Json(request):axum::Json<serde_json::Value>| {
        let count=inner.clone();
        async move {
            assert_eq!(headers.get("x-api-key").unwrap(),"fixture-key");
            assert_eq!(headers.get("anthropic-version").unwrap(),"2023-06-01");
            assert!(headers.get("authorization").is_none());
            assert!(headers.get("x-client-request-id").is_some());
            assert_eq!(request["thinking"]["type"],"disabled");
            assert_eq!(request["stream"],false);
            assert!(request.get("previous_response_id").is_none());
            count.fetch_add(1,Ordering::SeqCst);
            if malformed {"invalid response"} else {r#"{"type":"message","role":"assistant","id":"msg_fixture","model":"fixture-model","stop_reason":"end_turn","content":[{"type":"text","text":"bounded Claude answer"}],"usage":{"input_tokens":4,"output_tokens":3}}"#}
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
        arrived: Arc::new(tokio::sync::Notify::new()),
        task,
    }
}

#[tokio::test]
#[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
async fn anthropic_uses_selected_transport_and_retained_uncertainty_without_resending_requires_loopback()
 {
    for malformed in [false, true] {
        let fixture = anthropic_fixture(malformed).await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ledger.sqlite");
        let state = worker(&path, &fixture.url, None);
        *state.provider.lock().unwrap() =
            Some(ResponsesClient::for_provider("anthropic", "fixture-key", &fixture.url).unwrap());
        let mut input = command(&state);
        let (identity, metadata, mut payload) = decode_command(&input.command_bytes).unwrap();
        payload.binding.as_mut().unwrap().execution_binding_revision =
            "anthropic-messages-v1".into();
        payload.frozen_configuration.as_mut().unwrap().provider = "anthropic".into();
        input.command_bytes = encode_command(&identity, &metadata, &payload).unwrap();
        let prepared = prepare(State(state.clone()), auth(&state), axum::Json(input))
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
        assert_eq!(result.provider_call.as_ref().unwrap().provider, "anthropic");
        if malformed {
            assert_eq!(result.failure_reason, "provider_uncertain");
            assert_eq!(result.provider_call.as_ref().unwrap().total_tokens, None);
        } else {
            assert_eq!(result.response_content, "bounded Claude answer");
            assert_eq!(result.provider_call.as_ref().unwrap().total_tokens, Some(7));
        }
        let restarted = worker(&path, &fixture.url, Some(&state));
        *restarted.provider.lock().unwrap() = None;
        let replay = send(
            State(restarted.clone()),
            auth(&restarted),
            axum::Json(sending(&prepared, true, "sending")),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(replay.response_content, result.response_content);
        assert_eq!(replay.failure_reason, result.failure_reason);
        assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    }
}
