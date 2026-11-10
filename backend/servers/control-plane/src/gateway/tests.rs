use super::*;
use axum::{
    body::Body,
    http::{Request, header},
};
use prost::Message as _;
use tower::ServiceExt as _;

#[tokio::test]
async fn local_boundary_rejects_rebinding_cross_origin_and_simple_posts()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = Gateway::new("127.0.0.1:3000".parse()?, "127.0.0.1:1".parse()?)?;
    let app = router(gateway);
    for (host, origin, fetch_site, method) in [
        (
            "attacker.example:3000",
            "http://127.0.0.1:3000",
            "same-origin",
            "GET",
        ),
        (
            "127.0.0.1:3000",
            "https://attacker.example",
            "cross-site",
            "GET",
        ),
        (
            "127.0.0.1:3000",
            "http://127.0.0.1:3000",
            "cross-site",
            "GET",
        ),
        (
            "127.0.0.1:3000",
            "http://127.0.0.1:3000",
            "same-origin",
            "POST",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .method(method)
                    .header("host", host)
                    .header("origin", origin)
                    .header("sec-fetch-site", fetch_site)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "127.0.0.1:3000")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()?
            .contains("frame-ancestors 'none'")
    );
    for path in [
        "/workspace/",
        "/workspace/index.html",
        "/app.mjs",
        "/style.css",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:3000")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert!(Gateway::new("0.0.0.0:3000".parse()?, "127.0.0.1:1".parse()?).is_err());
    assert!(Gateway::new("127.0.0.1:3000".parse()?, "192.0.2.1:50051".parse()?).is_err());
    Ok(())
}

#[test]
fn protojson_keeps_64_bit_revisions_and_typed_expiry() -> Result<(), Box<dyn std::error::Error>> {
    let value = v1::ConversationSnapshot {
        conversation_revision: 9_007_199_254_740_993,
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(value)?["conversationRevision"],
        "9007199254740993"
    );
    let detail = v1::ErrorDetail {
        reason: v1::ErrorReason::WatchCursorExpired as i32,
        ..Default::default()
    };
    let status = Status::with_details(
        Code::FailedPrecondition,
        "private content",
        RichStatus {
            code: 9,
            message: String::new(),
            details: vec![prost_types::Any {
                type_url: "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail".into(),
                value: detail.encode_to_vec(),
            }],
        }
        .encode_to_vec()
        .into(),
    );
    assert_eq!(
        error_body(&status)["reason"],
        "ERROR_REASON_WATCH_CURSOR_EXPIRED"
    );
    assert!(!error_body(&status).to_string().contains("private content"));
    Ok(())
}
