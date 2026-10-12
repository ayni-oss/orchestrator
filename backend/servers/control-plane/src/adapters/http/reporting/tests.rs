use super::*;
use axum::{body::Body, extract::Json as ExtractJson, http::Request};
use serde_json::{Value, json};
use tower::ServiceExt as _;

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one receipt scenario owns authentication API fixtures and durable restart"
)]
async fn ingress_authenticates_before_durable_ack_and_preserves_receipts_on_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let event: Value = serde_json::from_slice(include_bytes!(
        "../../../../../../../proto/testdata/verification/report.json"
    ))?;
    let binding = &event["data"]["binding"];
    let admission = json!({"configuration":{"endpoint":"http://control-plane/events/verification","source":event["source"]},"binding":binding});
    let pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"pod-1","uid":binding["podUid"],"ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":binding["executionId"],"uid":"job-uid","controller":true}]},"spec":{"containers":[],"serviceAccountName":"runtime-coding"}});
    let job = json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":binding["executionId"],"uid":"job-uid"}});
    let map = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":binding["executionId"]},"immutable":true,"data":{"reporting.json":admission.to_string()}});
    let uid = binding["podUid"].clone();
    let api = Router::new()
        .route("/apis/authentication.k8s.io/v1/tokenreviews",post(move |ExtractJson(body): ExtractJson<Value>| {
            let uid = uid.clone(); async move { Json(json!({"apiVersion":"authentication.k8s.io/v1","kind":"TokenReview","spec":{},"status":{"authenticated":body["spec"]["token"] == "test-pod-token","audiences":[ayni_orchestrator_runtime::verification::REPORTING_AUDIENCE],"user":{"username":"system:serviceaccount:test:runtime-coding","extra":{"authentication.kubernetes.io/pod-name":["pod-1"],"authentication.kubernetes.io/pod-uid":[uid]}}}})) }
        }))
        .route("/api/v1/namespaces/test/pods/pod-1",axum::routing::get(move || { let pod = pod.clone(); async move { Json(pod) } }))
        .route("/apis/batch/v1/namespaces/test/jobs/verification-1",axum::routing::get(move || { let job = job.clone(); async move { Json(job) } }))
        .route("/api/v1/namespaces/test/configmaps/verification-1",axum::routing::get(move || { let map = map.clone(); async move { Json(map) } }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let config = kube::Config::new(format!("http://{}", listener.local_addr()?).parse()?);
    let server = tokio::spawn(async move { axum::serve(listener, api).await });
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("inbox.sqlite");
    let state = AppState::open(&path, 3)?;
    let authority = KubernetesAuthority {
        client: kube::Client::try_from(config)?,
        namespace: "test".into(),
        source: event["source"].as_str().ok_or("source")?.into(),
    };
    let service = Router::new()
        .route("/events/verification", post(receive))
        .layer(DefaultBodyLimit::max(
            ayni_orchestrator_events::MAX_EVENT_BYTES,
        ))
        .with_state(ReportingState {
            state: state.clone(),
            authority,
            capacity: std::sync::Arc::new(tokio::sync::Semaphore::new(32)),
        });
    let request = |body: Vec<u8>, token: &str| {
        Request::builder()
            .method("POST")
            .uri("/events/verification")
            .header("content-type", "application/cloudevents+json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body))
    };
    assert_eq!(
        service
            .clone()
            .oneshot(request(serde_json::to_vec(&event)?, "wrong-token")?)
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        state
            .database
            .execute(|store| store.pending_verifications())
            .await?
            .len(),
        0
    );
    assert_eq!(
        service
            .clone()
            .oneshot(request(
                vec![b' '; ayni_orchestrator_events::MAX_EVENT_BYTES + 1],
                "test-pod-token"
            )?)
            .await?
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for _ in 0..2 {
        let response = service
            .clone()
            .oneshot(request(serde_json::to_vec(&event)?, "test-pod-token")?)
            .await?;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let mut wrong = event.clone();
    wrong["data"]["binding"]["preparationId"] = json!("other");
    wrong["operationid"] = json!("other");
    assert_eq!(
        service
            .clone()
            .oneshot(request(serde_json::to_vec(&wrong)?, "test-pod-token")?)
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut conflict = event.clone();
    conflict["data"]["reportAvailable"]["evidence"]["sha256"] =
        json!(format!("sha256:{}", "f".repeat(64)));
    assert_eq!(
        service
            .clone()
            .oneshot(request(serde_json::to_vec(&conflict)?, "test-pod-token")?)
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    state.shutdown_database().await?;
    let reopened = AppState::open(&path, 3)?;
    assert_eq!(
        reopened
            .database
            .execute(|store| store.pending_verifications())
            .await?
            .len(),
        1
    );
    reopened.shutdown_database().await?;
    server.abort();
    Ok(())
}
