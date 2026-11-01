//! Public configuration references only; this test never accesses signing material.
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    store::runtime_signers::{SaveSigner, Signer},
};
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
fn key() -> String {
    uuid::Uuid::now_v7().to_string()
}
const fn input<'a>(key: &'a str, public: &'a str, uid: &'a str) -> SaveSigner<'a> {
    SaveSigner {
        key,
        id: "",
        expected_revision: 0,
        display_name: "Runtime public configuration",
        key_id: "runtime-signing-v1",
        public_key: public,
        secret_name: "ayni-runtime-signer-fixture",
        secret_uid: uid,
    }
}
#[allow(clippy::too_many_lines)]
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn public_signer_configuration_retains_exact_replay_revisions_and_access_boundaries()
-> TestResult {
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &role,
    )
    .await?;
    let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::validate_for_runtime(&url, &role).await?;
    let store = WorkflowStore::new(url.clone());
    let command = key();
    let public = "a".repeat(64);
    let uid = uuid::Uuid::new_v4().to_string();
    let request = input(&command, &public, &uid);
    let original = store.save_runtime_signer(request.clone()).await?;
    assert!(original.enabled);
    assert_eq!(original.revision, 1);
    assert_eq!(store.save_runtime_signer(request.clone()).await?, original);
    let restarted = WorkflowStore::new(url.clone());
    assert_eq!(
        restarted.get_runtime_signer_receipt(&command).await?,
        original
    );
    assert_eq!(restarted.get_runtime_signer(&original.id).await?, original);
    for altered in [
        SaveSigner {
            public_key: &"b".repeat(64),
            ..request.clone()
        },
        SaveSigner {
            secret_uid: &uuid::Uuid::new_v4().to_string(),
            ..request.clone()
        },
    ] {
        assert!(matches!(
            store.save_runtime_signer(altered).await,
            Err(StoreError::IdempotencyConflict)
        ));
    }
    for altered in [
        SaveSigner {
            key: &key(),
            public_key: &"A".repeat(64),
            ..request.clone()
        },
        SaveSigner {
            key: &key(),
            secret_name: "unscoped-name",
            ..request.clone()
        },
        SaveSigner {
            key: &key(),
            secret_name: "ayni-runtime-signer-trailing-",
            ..request.clone()
        },
        SaveSigner {
            key: &key(),
            secret_uid: "invalid",
            ..request.clone()
        },
        SaveSigner {
            key: &key(),
            key_id: "unsafe/id",
            ..request.clone()
        },
        SaveSigner {
            key: &key(),
            expected_revision: 1,
            ..request.clone()
        },
    ] {
        assert!(store.save_runtime_signer(altered).await.is_err());
    }
    let rotate = key();
    let changed = "b".repeat(64);
    let changed_uid = uuid::Uuid::new_v4().to_string();
    let rotated = store
        .save_runtime_signer(SaveSigner {
            key: &rotate,
            id: &original.id,
            expected_revision: 1,
            public_key: &changed,
            secret_name: "ayni-runtime-signer-replacement",
            secret_uid: &changed_uid,
            ..request.clone()
        })
        .await?;
    assert_eq!(rotated.revision, 2);
    assert_ne!(rotated.active_revision, original.active_revision);
    assert_eq!(rotated.public_key, changed);
    assert_eq!(store.save_runtime_signer(request.clone()).await?, original);
    assert_eq!(store.get_runtime_signer_receipt(&command).await?, original);
    assert_eq!(store.get_runtime_signer(&original.id).await?, rotated);
    assert!(matches!(
        store
            .save_runtime_signer(SaveSigner {
                key: &key(),
                id: &original.id,
                expected_revision: 1,
                ..request.clone()
            })
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    let disable = key();
    let disabled = store
        .set_runtime_signer_enabled(&disable, &original.id, 2, false)
        .await?;
    assert!(!disabled.enabled);
    assert_eq!(disabled.revision, 3);
    assert_ne!(disabled.active_revision, rotated.active_revision);
    assert_eq!(
        store
            .set_runtime_signer_enabled(&disable, &original.id, 2, false)
            .await?,
        disabled
    );
    assert!(matches!(
        store
            .set_runtime_signer_enabled(&disable, &original.id, 2, true)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .set_runtime_signer_enabled(&key(), &original.id, 2, true)
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    let rotated_disabled = store
        .save_runtime_signer(SaveSigner {
            key: &key(),
            id: &original.id,
            expected_revision: 3,
            ..request.clone()
        })
        .await?;
    assert!(!rotated_disabled.enabled);
    assert_eq!(rotated_disabled.revision, 4);
    let enabled = store
        .set_runtime_signer_enabled(&key(), &original.id, 4, true)
        .await?;
    assert!(enabled.enabled);
    assert_eq!(enabled.revision, 5);
    assert_eq!(store.get_runtime_signer_receipt(&disable).await?, disabled);
    let other = store
        .save_runtime_signer(SaveSigner {
            key: &key(),
            ..request
        })
        .await?;
    let (first, next) = store.list_runtime_signers("", 1).await?;
    assert_eq!(first.len(), 1);
    assert!(!next.is_empty());
    let (remaining, end) = store.list_runtime_signers(&next, 100).await?;
    assert!(end.is_empty());
    assert!(remaining.iter().all(|s| s.id != first[0].id));
    assert!(first.iter().chain(&remaining).any(|s| s.id == other.id));
    assert!(first.iter().chain(&remaining).any(|s| s.id == original.id));
    assert!(store.list_runtime_signers("", 101).await.is_err());
    assert!(store.list_runtime_signers("invalid", 50).await.is_err());
    let (client, driver) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
    let task = tokio::spawn(driver);
    let revisions = client
        .query(
            "SELECT configuration FROM workflow_runtime_signer_revisions WHERE signer_id=$1",
            &[&original.id],
        )
        .await?;
    let historical = revisions
        .iter()
        .map(|r| serde_json::from_str::<Signer>(r.get::<_, &str>(0)))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(historical.len(), 5);
    assert!(historical.contains(&original));
    assert!(historical.contains(&rotated));
    assert!(historical.contains(&disabled));
    assert!(historical.contains(&enabled));
    for sql in [
        "DELETE FROM workflow_runtime_signers",
        "UPDATE workflow_runtime_signers SET id='rewritten'",
        "DELETE FROM workflow_runtime_signer_revisions",
        "UPDATE workflow_runtime_signer_revisions SET configuration='rewritten'",
        "DELETE FROM workflow_runtime_signer_receipts",
        "UPDATE workflow_runtime_signer_receipts SET result='rewritten'",
    ] {
        let error = client
            .execute(sql, &[])
            .await
            .err()
            .ok_or("unexpected mutation privilege")?;
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }
    task.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn signer_key_name_corrections_preserve_key_material_and_original_receipts() -> TestResult {
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &role,
    )
    .await?;
    let store = WorkflowStore::new(std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?);
    let command = key();
    let public = "a".repeat(64);
    let uid = uuid::Uuid::new_v4().to_string();
    let request = SaveSigner {
        key_id: "old-fingerprint",
        ..input(&command, &public, &uid)
    };
    let original = store.save_runtime_signer(request.clone()).await?;
    let rename = key();
    let correction = SaveSigner {
        key: &rename,
        id: &original.id,
        expected_revision: 1,
        key_id: "project-2026",
        display_name: "project-2026",
        ..request.clone()
    };
    let named = store.save_runtime_signer(correction.clone()).await?;
    assert_eq!(named.key_id, "project-2026");
    assert_eq!(named.public_key, original.public_key);
    assert_eq!(named.secret_name, original.secret_name);
    assert_eq!(named.secret_uid, original.secret_uid);
    assert_eq!(named.revision, 2);
    assert_ne!(named.active_revision, original.active_revision);
    assert_eq!(store.save_runtime_signer(correction.clone()).await?, named);
    assert_eq!(store.save_runtime_signer(request).await?, original);
    assert_eq!(store.get_runtime_signer_receipt(&command).await?, original);
    assert_eq!(store.get_runtime_signer_receipt(&rename).await?, named);
    assert_eq!(store.get_runtime_signer(&original.id).await?, named);
    assert!(matches!(
        store
            .save_runtime_signer(SaveSigner {
                key: &key(),
                key_id: "another-name",
                ..correction.clone()
            })
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    assert!(matches!(
        store
            .save_runtime_signer(SaveSigner {
                key_id: "another-name",
                ..correction
            })
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    Ok(())
}
