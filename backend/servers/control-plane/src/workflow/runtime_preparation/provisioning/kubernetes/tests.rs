use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    response::IntoResponse as _,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
type Error = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone)]
struct Mock {
    op: SignerProvisioning,
    exists: bool,
    status: u16,
    post_status: u16,
    public_key: String,
    gets: u32,
    posts: u32,
    delay: bool,
    full: bool,
}
fn operation() -> SignerProvisioning {
    let id = uuid::Uuid::new_v4();
    SignerProvisioning {
        id: id.to_string(),
        idempotency_key: uuid::Uuid::new_v4().to_string(),
        namespace: "fixture-runtime".into(),
        display_name: "Fixture".into(),
        key_id: "fixture".into(),
        mode: "generate".into(),
        import_public_key: String::new(),
        secret_name: format!("ayni-runtime-signer-{}", id.simple()),
        state: "pending".into(),
        safe_reason: String::new(),
        public_key: String::new(),
        secret_uid: String::new(),
        signer: None,
        accepted_at_unix_ms: 1,
        updated_at_unix_ms: 1,
    }
}
fn metadata(state: &Mock) -> Value {
    json!({"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"namespace":"fixture-runtime","name":state.op.secret_name,"uid":"ad39eacd-4e47-49b9-ab7a-10d374b24e4d","annotations":annotations(&state.op,&state.public_key)}})
}
async fn observed(
    State(state): State<Arc<Mutex<Mock>>>,
    headers: HeaderMap,
) -> axum::response::Response {
    if headers.get("accept").and_then(|v| v.to_str().ok()) != Some(METADATA_ACCEPT) {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    let (status, body, delay) = {
        let Ok(mut state) = state.lock() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        state.gets += 1;
        let status = if state.status != 200 {
            state.status
        } else if !state.exists {
            404
        } else {
            200
        };
        let body = if state.full {
            json!({"apiVersion":"v1","kind":"Secret","metadata":{},"data":{"seed":"synthetic-echo"}})
        } else {
            metadata(&state)
        };
        (status, body, state.delay)
    };
    if delay {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(body),
    )
        .into_response()
}
async fn created(
    State(state): State<Arc<Mutex<Mock>>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let Ok(mut state) = state.lock() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    state.posts += 1;
    if body.get("immutable") != Some(&Value::Bool(true))
        || body.get("kind").and_then(Value::as_str) != Some("Secret")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state.exists = true;
    // Echoing a private payload on a failed POST must never affect diagnostics.
    (
        StatusCode::from_u16(state.post_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(body),
    )
        .into_response()
}
async fn fixture() -> Result<(Kubernetes, Arc<Mutex<Mock>>, tokio::task::JoinHandle<()>), Error> {
    let op = operation();
    let state = Arc::new(Mutex::new(Mock {
        op,
        exists: false,
        status: 200,
        post_status: 201,
        public_key: "a".repeat(64),
        gets: 0,
        posts: 0,
        delay: false,
        full: false,
    }));
    let router = Router::new()
        .route(
            "/api/v1/namespaces/fixture-runtime/secrets/{name}",
            get(observed),
        )
        .route("/api/v1/namespaces/fixture-runtime/secrets", post(created))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let client = Kubernetes {
        client: Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(70))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        origin,
        namespace: "fixture-runtime".into(),
        token: Zeroizing::new("synthetic-token".into()),
    };
    Ok((client, state, task))
}
#[tokio::test]
async fn generation_only_follows_actual_absence_and_adopts_atomic_create_winner()
-> Result<(), Error> {
    for status in [201, 409, 500] {
        let (client, state, task) = fixture().await?;
        state.lock().map_err(|_| "lock")?.post_status = status;
        let op = state.lock().map_err(|_| "lock")?.op.clone();
        let result = super::super::execute_with(&client, &op, None).await?;
        assert_eq!(result.1, "a".repeat(64));
        assert_eq!(state.lock().map_err(|_| "lock")?.posts, 1);
        task.abort();
    }
    Ok(())
}
#[tokio::test]
async fn access_denial_outage_bad_metadata_and_timeout_never_generate() -> Result<(), Error> {
    for status in [401, 403, 406, 429, 500, 503] {
        let (client, state, task) = fixture().await?;
        state.lock().map_err(|_| "lock")?.status = status;
        let op = state.lock().map_err(|_| "lock")?.op.clone();
        assert!(
            super::super::execute_with(&client, &op, None)
                .await
                .is_err()
        );
        assert_eq!(state.lock().map_err(|_| "lock")?.posts, 0);
        task.abort();
    }
    for full in [true, false] {
        let (client, state, task) = fixture().await?;
        {
            let mut value = state.lock().map_err(|_| "lock")?;
            value.exists = true;
            value.full = full;
            value.delay = !full;
        }
        let op = state.lock().map_err(|_| "lock")?.op.clone();
        assert!(
            super::super::execute_with(&client, &op, None)
                .await
                .is_err()
        );
        assert_eq!(state.lock().map_err(|_| "lock")?.posts, 0);
        task.abort();
    }
    Ok(())
}
#[tokio::test]
async fn import_recovery_needs_same_material_and_foreign_metadata_is_rejected() -> Result<(), Error>
{
    let (client, state, task) = fixture().await?;
    let mut op = state.lock().map_err(|_| "lock")?.op.clone();
    op.mode = "import".into();
    op.import_public_key = "b".repeat(64);
    assert_eq!(
        super::super::execute_with(&client, &op, None).await,
        Err("signer_provisioning_inputs_required")
    );
    assert_eq!(state.lock().map_err(|_| "lock")?.posts, 0);
    {
        let mut value = state.lock().map_err(|_| "lock")?;
        value.exists = true;
        value.op = op.clone();
    }
    assert_eq!(client.observed(&op).await, Err(CONFLICT));
    {
        let mut value = state.lock().map_err(|_| "lock")?;
        value.public_key = op.import_public_key.clone();
    }
    assert!(client.observed(&op).await?.is_some());
    let mut body = {
        let value = state.lock().map_err(|_| "lock")?;
        metadata(&value)
    };
    for field in ["namespace", "name", "uid"] {
        let original = body["metadata"][field].clone();
        body["metadata"][field] = json!("foreign");
        assert!(validate_observed(&serde_json::to_vec(&body)?, "fixture-runtime", &op).is_err());
        body["metadata"][field] = original;
    }
    body["metadata"]["annotations"]["ayni.build/signer-provisioning"] =
        json!(uuid::Uuid::new_v4().to_string());
    assert!(validate_observed(&serde_json::to_vec(&body)?, "fixture-runtime", &op).is_err());
    task.abort();
    Ok(())
}
