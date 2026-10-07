#![allow(
    clippy::expect_used,
    reason = "test fixture and negative-path failures intentionally stop the test"
)]
use super::*;
fn state() -> (tempfile::TempDir, AppState) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let state = AppState::open(directory.path().join("state.db"), 3)
        .expect("store")
        .with_github_onboarding();
    (directory, state)
}
#[tokio::test]
async fn deployment_without_github_has_empty_connections() {
    let (_directory, state) = state();
    assert!(state.connections().await.expect("list").is_empty());
    assert!(
        state
            .source
            .as_ref()
            .expect("shared provider")
            .validate_app()
            .await
            .is_err()
    );
}
#[tokio::test]
async fn admin_credentials_cannot_be_replaced_or_disabled() {
    let (_directory, state) = state();
    let connection = v1::Connection {
        id: "admin".into(),
        kind: "github_app".into(),
        management_source: "admin".into(),
        resource_version: 1,
        enabled: true,
        ..Default::default()
    };
    state
        .persist_connection(connection)
        .await
        .expect("admin registration");
    let error = state
        .replace_connection(v1::ReplaceConnectionCredentialsRequest {
            id: "admin".into(),
            idempotency_key: Uuid::now_v7().to_string(),
            expected_resource_version: 1,
            credentials: Some(v1::ConnectionCredentials {
                private_key: "sensitive-value".into(),
                ..Default::default()
            }),
        })
        .await
        .expect_err("readonly admin");
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    let error = state
        .enable_connection("admin".into(), 1, false, Uuid::now_v7().to_string())
        .await
        .expect_err("readonly admin");
    assert_eq!(error.code(), tonic::Code::PermissionDenied);
    assert_eq!(
        state
            .connection("admin".into())
            .await
            .expect("retained")
            .resource_version,
        1
    );
}
#[tokio::test]
async fn invalid_private_key_does_not_create_metadata_or_receipt() {
    let (_directory, state) = state();
    let error = state
        .create_connection(v1::CreateConnectionRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            kind: "github_app".into(),
            display_name: "App".into(),
            app_id: "1".into(),
            credentials: Some(v1::ConnectionCredentials {
                private_key: "sensitive-invalid-key".into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect_err("invalid key");
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(!error.message().contains("sensitive"));
    assert!(state.connections().await.expect("list").is_empty());
}
#[tokio::test]
async fn stale_connection_version_rejects_rotation_before_secret_access() {
    let (_directory, state) = state();
    state
        .persist_connection(v1::Connection {
            id: "ui".into(),
            kind: "github_app".into(),
            management_source: "ui".into(),
            resource_version: 2,
            ..Default::default()
        })
        .await
        .expect("record");
    let error = state
        .replace_connection(v1::ReplaceConnectionCredentialsRequest {
            id: "ui".into(),
            expected_resource_version: 1,
            idempotency_key: Uuid::now_v7().to_string(),
            ..Default::default()
        })
        .await
        .expect_err("stale");
    assert_eq!(error.code(), tonic::Code::Aborted);
}
#[tokio::test]
async fn idempotency_key_cannot_change_safe_connection_identity() {
    let (_directory, state) = state();
    let key = Uuid::now_v7().to_string();
    let id = format!("connection-{:x}", Sha256::digest(key.as_bytes()));
    state
        .persist_connection(v1::Connection {
            id: id[..43].into(),
            kind: "github_app".into(),
            management_source: "ui".into(),
            display_name: "First".into(),
            app_id: "1".into(),
            resource_version: 1,
            ..Default::default()
        })
        .await
        .expect("record");
    let error = state
        .create_connection(v1::CreateConnectionRequest {
            idempotency_key: key,
            kind: "github_app".into(),
            display_name: "Changed".into(),
            app_id: "1".into(),
            ..Default::default()
        })
        .await
        .expect_err("safe identity conflict");
    assert_eq!(error.code(), tonic::Code::AlreadyExists);
}

#[tokio::test]
async fn concurrent_app_registration_has_one_durable_winner() {
    let (_directory, state) = state();
    let first = v1::Connection {
        id: "first".into(),
        kind: "github_app".into(),
        management_source: "admin".into(),
        resource_version: 1,
        ..Default::default()
    };
    let second = v1::Connection {
        id: "second".into(),
        kind: "github_app".into(),
        management_source: "ui".into(),
        resource_version: 1,
        ..Default::default()
    };
    let (first, second) = tokio::join!(
        state.persist_connection(first),
        state.persist_connection(second)
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(state.connections().await.expect("list").len(), 1);
}

#[tokio::test]
async fn deployment_admin_registration_cannot_adopt_ui_credentials() {
    let (_directory, state) = state();
    state
        .persist_connection(v1::Connection {
            id: "ui".into(),
            kind: "github_app".into(),
            management_source: "ui".into(),
            secret_name: "ui-owned-secret".into(),
            secret_uid: "ui-uid".into(),
            resource_version: 1,
            enabled: true,
            status: "ready".into(),
            ..Default::default()
        })
        .await
        .expect("record");
    let error = state
        .register_admin_github(Some("admin-secret".into()))
        .await
        .expect_err("ownership conflict");
    assert_eq!(error.message(), "github_admin_identity_conflict");
    let connection = state
        .connection("ui".into())
        .await
        .expect("preserved record");
    assert_eq!(connection.management_source, "ui");
    assert_eq!(connection.secret_name, "ui-owned-secret");
    assert_eq!(connection.secret_uid, "ui-uid");
    assert_eq!(connection.failure_reason, "github_admin_identity_conflict");
    assert_eq!(state.connections().await.expect("list").len(), 1);
}
