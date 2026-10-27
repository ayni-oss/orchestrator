//! Synthetic credential delivery uses the same signed claim and exact-command authority.
use super::{Fixture, TestResult, key};
use ayni_orchestrator_workflow_postgres::store::connections::{ProviderSettings, SaveProvider};

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with the runtime-role URL"]
async fn scoped_credential_delivery_requires_claim_and_preserves_the_admitted_revision()
-> TestResult {
    let fixture = Fixture::new().await?;
    let save = fixture
        .store
        .save_provider_connection(SaveProvider {
            key: &key(),
            id: "",
            expected: 0,
            name: "Scoped OpenAI",
            provider: "openai",
            capability: "llm",
            settings: ProviderSettings::default(),
            credential: "fixture-frozen-credential",
        })
        .await?;
    fixture
        .store
        .finish_provider_validation(&save.id, "", &["explicit-fixture-model".into()])
        .await?;
    let current = fixture
        .store
        .get_agent_configuration(&fixture.workspace, "")
        .await?;
    let mut proposed = current.clone();
    proposed.revision_id.clear();
    proposed.connection_id = save.connection_id.clone();
    fixture
        .store
        .update_agent_configuration(&key(), &current.revision_id, &proposed)
        .await?;
    let turn = fixture.send().await?;
    let bytes: Vec<u8> = fixture
        .client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&turn],
        )
        .await?
        .get(0);
    assert!(
        fixture
            .store
            .session_credential(bytes.clone())
            .await
            .is_err()
    );
    let call = fixture.claim(&turn).await?;
    assert_eq!(
        fixture.store.session_credential(call.bytes.clone()).await?,
        "fixture-frozen-credential"
    );
    let rotation = fixture
        .store
        .save_provider_connection(SaveProvider {
            key: &key(),
            id: &save.connection_id,
            expected: 1,
            name: "Scoped OpenAI",
            provider: "openai",
            capability: "llm",
            settings: ProviderSettings::default(),
            credential: "fixture-rotated-credential",
        })
        .await?;
    fixture
        .store
        .finish_provider_validation(&rotation.id, "", &["explicit-fixture-model".into()])
        .await?;
    fixture
        .store
        .set_connection_enabled(&key(), &save.connection_id, 2, false)
        .await?;
    assert_eq!(
        fixture.store.session_credential(call.bytes.clone()).await?,
        "fixture-frozen-credential"
    );
    assert!(
        !bytes
            .windows(b"fixture-frozen-credential".len())
            .any(|v| v == b"fixture-frozen-credential")
    );
    let mut changed = call.bytes.clone();
    changed.push(0);
    assert!(fixture.store.session_credential(changed).await.is_err());
    fixture.cancel(&turn, &key()).await?;
    assert!(fixture.store.session_credential(call.bytes).await.is_err());
    fixture.driver.abort();
    Ok(())
}
