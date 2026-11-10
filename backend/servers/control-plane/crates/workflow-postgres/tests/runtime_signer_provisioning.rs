//! Public provisioning authority only; fixtures contain no private signing material.
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    store::{runtime_signer_provisioning::BeginSignerProvisioning, runtime_signers::SaveSigner},
};
use tokio_postgres::{Client, NoTls};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn key() -> String {
    uuid::Uuid::now_v7().to_string()
}
const fn input(key: &str) -> BeginSignerProvisioning<'_> {
    BeginSignerProvisioning {
        key,
        display_name: "Provisioned fixture",
        namespace: "runtime-fixture",
        key_id: "fixture-v1",
        mode: "generate",
        import_public_key: "",
    }
}
async fn setup() -> TestResult<(WorkflowStore, Client)> {
    let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &role,
    )
    .await?;
    let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::validate_for_runtime(&url, &role).await?;
    let (client, driver) = tokio_postgres::connect(&url, NoTls).await?;
    tokio::spawn(async move {
        let _ = driver.await;
    });
    Ok((WorkflowStore::new(url), client))
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL"]
#[allow(clippy::too_many_lines)]
async fn provisioning_replays_frozen_public_result_and_reserves_manual_command_keys() -> TestResult
{
    let (store, client) = setup().await?;
    let reserved_key = key();
    assert!(matches!(
        store
            .begin_runtime_signer_provisioning(BeginSignerProvisioning {
                key_id: "unsigned",
                ..input(&reserved_key)
            })
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store.get_runtime_signer_provisioning(&reserved_key).await,
        Err(StoreError::NotFound)
    ));
    let reserved_public = "a".repeat(64);
    let reserved_uid = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        store
            .save_runtime_signer(SaveSigner {
                key: &reserved_key,
                id: "",
                expected_revision: 0,
                display_name: "Reserved",
                key_id: "unsigned",
                public_key: &reserved_public,
                secret_name: "ayni-runtime-signer-reserved",
                secret_uid: &reserved_uid
            })
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store.get_runtime_signer_receipt(&reserved_key).await,
        Err(StoreError::NotFound)
    ));
    let command = key();
    let request = input(&command);
    let first = store
        .begin_runtime_signer_provisioning(request.clone())
        .await?;
    assert_eq!(first.state, "pending");
    assert_eq!(first.namespace, "runtime-fixture");
    assert!(first.public_key.is_empty());
    assert!(first.secret_uid.is_empty());
    assert_eq!(
        first.secret_name,
        format!("ayni-runtime-signer-{}", first.id.replace('-', ""))
    );
    let restarted = WorkflowStore::new(std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?);
    assert_eq!(
        restarted.get_runtime_signer_provisioning(&command).await?,
        first
    );
    for changed in [
        BeginSignerProvisioning {
            namespace: "other-runtime",
            ..request.clone()
        },
        BeginSignerProvisioning {
            display_name: "Changed",
            ..request.clone()
        },
        BeginSignerProvisioning {
            key_id: "changed",
            ..request.clone()
        },
        BeginSignerProvisioning {
            mode: "import",
            import_public_key: &"b".repeat(64),
            ..request.clone()
        },
    ] {
        assert!(matches!(
            store.begin_runtime_signer_provisioning(changed).await,
            Err(StoreError::IdempotencyConflict)
        ));
    }
    let waiting = store
        .note_runtime_signer_provisioning_failure(&command, "signer_provisioning_inputs_required")
        .await?;
    assert_eq!(waiting.state, "awaiting_material");
    assert_eq!(waiting.secret_name, first.secret_name);
    let denied = store
        .note_runtime_signer_provisioning_failure(&command, "signer_provisioning_access_denied")
        .await?;
    assert_eq!(denied.state, "pending");
    assert_eq!(
        store.begin_runtime_signer_provisioning(request).await?,
        denied
    );
    assert!(matches!(
        store
            .complete_runtime_signer_provisioning(&command, "", &"a".repeat(64))
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    let uid = uuid::Uuid::new_v4().to_string();
    let public = "a".repeat(64);
    let completed = store
        .complete_runtime_signer_provisioning(&command, &uid, &public)
        .await?;
    let signer = completed.signer.clone().ok_or("missing signer")?;
    assert_eq!(signer.revision, 1);
    assert!(signer.enabled);
    assert_eq!(completed.state, "completed");
    let disabled = store
        .set_runtime_signer_enabled(&key(), &signer.id, 1, false)
        .await?;
    assert!(!disabled.enabled);
    assert_eq!(
        restarted
            .complete_runtime_signer_provisioning(&command, &uid, &public)
            .await?,
        completed
    );
    assert_eq!(
        restarted.get_runtime_signer_receipt(&command).await?,
        signer
    );
    assert_eq!(
        store
            .note_runtime_signer_provisioning_failure(&command, "signer_provisioning_unavailable")
            .await?,
        completed
    );
    for (altered_uid, altered_public) in [
        (uuid::Uuid::new_v4().to_string(), public.clone()),
        (uid.clone(), "b".repeat(64)),
    ] {
        assert!(matches!(
            store
                .complete_runtime_signer_provisioning(&command, &altered_uid, &altered_public)
                .await,
            Err(StoreError::IdempotencyConflict)
        ));
    }
    let manual = SaveSigner {
        key: &command,
        id: "",
        expected_revision: 0,
        display_name: "Manual",
        key_id: "fixture-manual",
        public_key: &public,
        secret_name: "ayni-runtime-signer-manual",
        secret_uid: &uid,
    };
    assert!(matches!(
        store.save_runtime_signer(manual.clone()).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .set_runtime_signer_enabled(&command, &signer.id, 2, true)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let manual_key = key();
    store
        .save_runtime_signer(SaveSigner {
            key: &manual_key,
            ..manual
        })
        .await?;
    assert!(matches!(
        store
            .begin_runtime_signer_provisioning(input(&manual_key))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(client.execute("UPDATE workflow_runtime_signer_provisioning SET key_id='changed' WHERE idempotency_key=$1",&[&command]).await.is_err());
    assert!(client.execute("UPDATE workflow_runtime_signer_provisioning SET namespace='other-runtime' WHERE idempotency_key=$1",&[&command]).await.is_err());
    assert!(
        client
            .execute(
                "DELETE FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1",
                &[&command]
            )
            .await
            .is_err()
    );
    assert!(client.execute("UPDATE workflow_runtime_signer_receipts SET result='changed' WHERE idempotency_key=$1",&[&command]).await.is_err());
    let safe:String=client.query_one("SELECT safe_request FROM workflow_runtime_signer_provisioning WHERE idempotency_key=$1",&[&command]).await?.get(0);
    let fields: serde_json::Value = serde_json::from_str(&safe)?;
    assert_eq!(fields.as_object().ok_or("request object")?.len(), 6);
    assert_eq!(fields["mode"], "generate");
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL"]
async fn concurrent_reservation_and_completion_retain_exactly_one_winner() -> TestResult {
    let (store, client) = setup().await?;
    let command = key();
    let (a, b) = tokio::join!(
        store.begin_runtime_signer_provisioning(input(&command)),
        store.begin_runtime_signer_provisioning(input(&command))
    );
    assert_eq!(a?, b?);
    let ua = uuid::Uuid::new_v4().to_string();
    let ub = uuid::Uuid::new_v4().to_string();
    let pa = "a".repeat(64);
    let pb = "b".repeat(64);
    let (a, b) = tokio::join!(
        store.complete_runtime_signer_provisioning(&command, &ua, &pa),
        store.complete_runtime_signer_provisioning(&command, &ub, &pb)
    );
    let ((Ok(winner), Err(StoreError::IdempotencyConflict))
    | (Err(StoreError::IdempotencyConflict), Ok(winner))) = (a, b)
    else {
        return Err("completion did not fence a unique winner".into());
    };
    let signer = winner.signer.clone().ok_or("missing signer")?;
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_runtime_signer_receipts WHERE idempotency_key=$1",
            &[&command],
        )
        .await?
        .get(0);
    assert_eq!(count, 1);
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_runtime_signer_revisions WHERE signer_id=$1",
            &[&signer.id],
        )
        .await?
        .get(0);
    assert_eq!(count, 1);
    let (note, again) = tokio::join!(
        store.note_runtime_signer_provisioning_failure(&command, "signer_provisioning_conflict"),
        store.complete_runtime_signer_provisioning(
            &command,
            &winner.secret_uid,
            &winner.public_key
        )
    );
    assert_eq!(note?, winner);
    assert_eq!(again?, winner);
    let collision_key = key();
    let manual = SaveSigner {
        key: &collision_key,
        id: "",
        expected_revision: 0,
        display_name: "Manual race",
        key_id: "manual-race",
        public_key: &pa,
        secret_name: "ayni-runtime-signer-manual-race",
        secret_uid: &ua,
    };
    let (reserve, save) = tokio::join!(
        store.begin_runtime_signer_provisioning(input(&collision_key)),
        store.save_runtime_signer(manual)
    );
    assert!(matches!(
        (reserve, save),
        (Ok(_), Err(StoreError::IdempotencyConflict))
            | (Err(StoreError::IdempotencyConflict), Ok(_))
    ));
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL"]
async fn import_recovery_requires_original_public_identity_and_never_becomes_generation()
-> TestResult {
    let (store, _) = setup().await?;
    let command = key();
    let public = "a".repeat(64);
    let request = BeginSignerProvisioning {
        mode: "import",
        import_public_key: &public,
        ..input(&command)
    };
    let pending = store
        .begin_runtime_signer_provisioning(request.clone())
        .await?;
    let missing = store
        .note_runtime_signer_provisioning_failure(&command, "signer_provisioning_inputs_required")
        .await?;
    assert_eq!(missing.mode, "import");
    assert_eq!(missing.state, "awaiting_material");
    assert_eq!(missing.secret_name, pending.secret_name);
    assert!(matches!(
        store
            .begin_runtime_signer_provisioning(input(&command))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .begin_runtime_signer_provisioning(BeginSignerProvisioning {
                import_public_key: &"b".repeat(64),
                ..request.clone()
            })
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let uid = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        store
            .complete_runtime_signer_provisioning(&command, &uid, &"b".repeat(64))
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(
        store.begin_runtime_signer_provisioning(request).await?,
        missing
    );
    assert_eq!(
        store
            .complete_runtime_signer_provisioning(&command, &uid, &public)
            .await?
            .state,
        "completed"
    );
    assert!(matches!(
        store.get_runtime_signer_provisioning(&key()).await,
        Err(StoreError::NotFound)
    ));
    Ok(())
}
