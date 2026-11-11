use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ayni_orchestrator_control_plane::gateway::{Gateway, development_router, router};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

#[tokio::test]
async fn dev_status_is_opt_in_guarded_and_available_without_backend()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = Gateway::new("127.0.0.1:3000".parse()?, "127.0.0.1:1".parse()?)?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("status.json");
    std::fs::write(&path, r#"{"phase":"migrating","checkout":"/workspace"}"#)?;
    let request = |host: &str| {
        Request::builder()
            .uri("/__dev/status")
            .header("host", host)
            .body(Body::empty())
    };
    let normal = router(gateway.clone())
        .oneshot(request("127.0.0.1:3000")?)
        .await?;
    assert_eq!(normal.status(), StatusCode::NOT_FOUND);
    let app = development_router(gateway, path);
    let forbidden = app.clone().oneshot(request("attacker.example")?).await?;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let response = app.oneshot(request("127.0.0.1:3000")?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
    assert_eq!(value["phase"], "migrating");
    Ok(())
}
