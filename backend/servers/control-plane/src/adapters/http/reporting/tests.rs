use super::*;
use axum::{body::Body, extract::Json as ExtractJson, http::Request};
use serde_json::{Value, json};
use tower::ServiceExt as _;

#[allow(
    clippy::too_many_lines,
    reason = "one receipt scenario owns authentication API fixtures and durable restart"
)]
async fn run_ingress_contract(mode: u8) -> Result<(), Box<dyn std::error::Error>> {
    let mut event: Value = serde_json::from_slice(include_bytes!(
        "../../../../../../../proto/testdata/verification/report.json"
    ))?;
    if mode == 1 {
        event["type"] = json!(ayni_orchestrator_events::operations::EVENT_TYPE);
        event["dataschema"] = json!(ayni_orchestrator_events::operations::DATA_SCHEMA);
        event["data"]["binding"]
            .as_object_mut()
            .ok_or("binding")?
            .remove("resolvedImageDigest");
        let evidence = event["data"]["reportAvailable"]["evidence"].clone();
        let binding = event["data"]["binding"].clone();
        event["data"] = json!({"binding":binding,"kind":"RUNTIME_OPERATION_KIND_DISCOVERY","evidence":evidence});
        event["data"]["evidence"]["evidenceId"] = json!("runtime-operation-json");
        event["data"]["evidence"]["contract"] =
            json!(ayni_orchestrator_events::operations::CONTRACT);
    }
    if mode == 2 {
        let original = event["data"]["binding"].clone();
        event["type"] = json!(ayni_orchestrator_events::coding::EVENT_TYPE);
        event["dataschema"] = json!(ayni_orchestrator_events::coding::DATA_SCHEMA);
        event["subject"] = json!("tasks/task-one/attempts/attempt-one");
        event["data"] = json!({"binding":{"taskId":"task-one","attemptId":"attempt-one","generation":original["preparationId"],"executionId":original["executionId"],"podUid":original["podUid"],"snapshotDigest":original["requestDigest"],"requestDigest":original["requestDigest"]},"kind":"CODING_REPORT_KIND_FINAL","sequence":257,"sha256":original["requestDigest"],"sizeBytes":100,"contract":ayni_orchestrator_events::coding::CONTRACT});
    }
    let binding = &event["data"]["binding"];
    let mut admission = json!({"configuration":{"endpoint":"http://control-plane/events/verification","source":event["source"]},"binding":binding});
    if mode == 1 {
        admission["kind"] = event["data"]["kind"].clone();
    }
    let manifest_key = if mode == 2 {
        "coding-reporting.json"
    } else if mode == 1 {
        "operation-reporting.json"
    } else {
        "reporting.json"
    };
    let pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"pod-1","uid":binding["podUid"],"ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":binding["executionId"],"uid":"job-uid","controller":true}]},"spec":{"containers":[],"serviceAccountName":"runtime-coding"}});
    let job = json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":binding["executionId"],"uid":"job-uid"}});
    let map = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":binding["executionId"]},"immutable":true,"data":{(manifest_key):admission.to_string()}});
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
            .execute(move |store| receipt_count(store, mode))
            .await?,
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
    wrong["data"]["binding"][if mode == 2 {
        "generation"
    } else {
        "preparationId"
    }] = json!("other");
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
    let evidence = if mode == 2 {
        &mut conflict["data"]
    } else if mode == 1 {
        &mut conflict["data"]["evidence"]
    } else {
        &mut conflict["data"]["reportAvailable"]["evidence"]
    };
    evidence["sha256"] = json!(format!("sha256:{}", "f".repeat(64)));
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
            .execute(move |store| receipt_count(store, mode))
            .await?,
        1
    );
    reopened.shutdown_database().await?;
    server.abort();
    Ok(())
}

fn receipt_count(
    store: &crate::adapters::sqlite::CleanStore,
    mode: u8,
) -> Result<usize, CleanStoreError> {
    if mode == 2 {
        Ok(store.pending_coding("attempt-one")?.len())
    } else if mode == 1 {
        Ok(usize::from(
            store.runtime_operation_receipt("verification-1")?.is_some(),
        ))
    } else {
        Ok(store.pending_verifications()?.len())
    }
}
#[tokio::test]
async fn ingress_authenticates_before_durable_ack_and_preserves_receipts_on_restart()
-> Result<(), Box<dyn std::error::Error>> {
    run_ingress_contract(0).await
}
#[tokio::test]
async fn operation_ingress_checks_manifest_binding_and_conflicts_across_restart()
-> Result<(), Box<dyn std::error::Error>> {
    run_ingress_contract(1).await
}

#[tokio::test]
async fn coding_ingress_authenticates_frozen_generation_and_rejects_conflicting_terminal_reports()
-> Result<(), Box<dyn std::error::Error>> {
    run_ingress_contract(2).await
}
