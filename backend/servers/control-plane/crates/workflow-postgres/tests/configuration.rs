use ayni_orchestrator_workflow_postgres::{StoreError, WorkflowStore, model::AgentConfiguration};
use std::error::Error;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with the runtime-role URL"]
#[allow(
    clippy::too_many_lines,
    reason = "the cumulative configuration scenario keeps admission and edit ordering explicit"
)]
async fn immutable_configuration_freezes_admission_and_isolates_conversations() -> TestResult {
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime_url.clone());
    let workspace = store
        .create_workspace(&key(), "Configuration workspace")
        .await?
        .resource;
    let project = store
        .create_project(&key(), &workspace.id, "Project", "configuration isolation")
        .await?
        .resource;
    let a = store
        .create_conversation(&key(), &workspace.id, &project.id, "A")
        .await?
        .resource;
    let b = store
        .create_conversation(&key(), &workspace.id, &project.id, "B")
        .await?
        .resource;
    let initial = store.get_agent_configuration(&workspace.id, "").await?;
    assert_eq!(initial.provider, "scripted");
    let first = store
        .send_message(&key(), &workspace.id, &project.id, &a.id, "Only A")
        .await?;
    let queued = store
        .send_message(&key(), &workspace.id, &project.id, &a.id, "Queued A")
        .await?;
    let first_inputs = store
        .get_turn_inputs(&workspace.id, &project.id, &a.id, &first.turn_id)
        .await?;
    assert_eq!(first_inputs.configuration, initial);
    let manifest = store
        .get_turn_context_inputs(&workspace.id, &project.id, &a.id, &first.turn_id, 0, 1)
        .await?;
    assert_eq!(manifest.manifest.record_count, 1);
    assert_eq!(manifest.records[0].content, "Only A");
    assert_eq!(manifest.next_after_ordinal, 0);
    assert!(matches!(
        store
            .get_turn_context_inputs(&workspace.id, &project.id, &b.id, &first.turn_id, 0, 1)
            .await,
        Err(StoreError::NotFound)
    ));
    let mut proposed = AgentConfiguration {
        revision_id: String::new(),
        instructions: "Shared workspace instructions".into(),
        provider: "openai".into(),
        model: "operator-selected-model".into(),
        ..initial.clone()
    };
    let update_key = key();
    let updated = store
        .update_agent_configuration(&update_key, &initial.revision_id, &proposed)
        .await?;
    assert_ne!(updated.revision_id, initial.revision_id);
    assert_eq!(
        store
            .update_agent_configuration(&update_key, &initial.revision_id, &proposed)
            .await?,
        updated
    );
    assert_eq!(
        store
            .get_agent_configuration(&workspace.id, &initial.revision_id)
            .await?,
        initial
    );
    assert_eq!(
        store
            .get_turn_inputs(&workspace.id, &project.id, &a.id, &queued.turn_id)
            .await?
            .configuration,
        initial
    );
    assert!(matches!(
        store
            .update_agent_configuration(&key(), &initial.revision_id, &proposed)
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    proposed.instructions = "Changed retry payload".into();
    assert!(matches!(
        store
            .update_agent_configuration(&update_key, &initial.revision_id, &proposed)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let sessions_a = store
        .conversation_sessions(&workspace.id, &project.id, &a.id)
        .await?;
    assert_eq!(sessions_a.len(), 1);
    assert!(matches!(
        store
            .replace_idle_session(
                &workspace.id,
                &project.id,
                &a.id,
                sessions_a[0].generation,
                &key()
            )
            .await,
        Err(StoreError::InvalidInput("session_has_retained_work"))
    ));
    let other = store
        .send_message(&key(), &workspace.id, &project.id, &b.id, "Only B")
        .await?;
    assert_eq!(
        store
            .get_turn_inputs(&workspace.id, &project.id, &b.id, &other.turn_id)
            .await?
            .configuration,
        updated
    );
    assert!(matches!(
        store
            .get_turn_inputs(&workspace.id, &project.id, &b.id, &first.turn_id)
            .await,
        Err(StoreError::NotFound)
    ));
    let sessions_b = store
        .conversation_sessions(&workspace.id, &project.id, &b.id)
        .await?;
    assert_ne!(sessions_a[0].id, sessions_b[0].id);
    let a_snapshot = store
        .get_snapshot(&workspace.id, &project.id, &a.id)
        .await?;
    assert!(
        a_snapshot
            .recent_messages
            .iter()
            .all(|m| m.content != "Only B")
    );
    let b_snapshot = store
        .get_snapshot(&workspace.id, &project.id, &b.id)
        .await?;
    assert!(
        b_snapshot
            .recent_messages
            .iter()
            .all(|m| !m.content.contains("Only A"))
    );
    assert!(
        b_snapshot
            .nonterminal_turns
            .iter()
            .all(|t| t.execution_binding_revision == "scripted-v1")
    );
    // Two concurrent edits from one expected identity produce one new revision.
    proposed.instructions = "Concurrent edit".into();
    let k1 = key();
    let k2 = key();
    let (one, two) = tokio::join!(
        store.update_agent_configuration(&k1, &updated.revision_id, &proposed),
        store.update_agent_configuration(&k2, &updated.revision_id, &proposed)
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    assert!(
        matches!(one, Err(StoreError::ConfigurationRevisionConflict))
            || matches!(two, Err(StoreError::ConfigurationRevisionConflict))
    );
    // Replaying admission after edits returns the original frozen input identity.
    let replay_key = key();
    let admitted = store
        .send_message(
            &replay_key,
            &workspace.id,
            &project.id,
            &a.id,
            "Replay exact",
        )
        .await?;
    let retained = store
        .get_turn_inputs(&workspace.id, &project.id, &a.id, &admitted.turn_id)
        .await?;
    let current = store.get_agent_configuration(&workspace.id, "").await?;
    proposed.instructions = "Later configuration".into();
    store
        .update_agent_configuration(&key(), &current.revision_id, &proposed)
        .await?;
    assert_eq!(
        store
            .send_message(
                &replay_key,
                &workspace.id,
                &project.id,
                &a.id,
                "Replay exact"
            )
            .await?,
        admitted
    );
    assert_eq!(
        store
            .get_turn_inputs(&workspace.id, &project.id, &a.id, &admitted.turn_id)
            .await?,
        retained
    );
    prove_large_context(&store, &runtime_url, &workspace.id, &project.id).await?;
    Ok(())
}

async fn prove_large_context(
    store: &WorkflowStore,
    runtime_url: &str,
    workspace_id: &str,
    project_id: &str,
) -> TestResult {
    // Large history retains every immutable reference while the event carries an explicit partial tail.
    let large = store
        .create_conversation(&key(), workspace_id, project_id, "Large context")
        .await?
        .resource;
    let (client, driver) = ayni_orchestrator_workflow_postgres::connect(runtime_url).await?;
    for _ in 0..51 {
        let receipt = store
            .send_message(
                &key(),
                workspace_id,
                project_id,
                &large.id,
                &"x".repeat(16_000),
            )
            .await?;
        complete_turn(store, &client, &receipt.turn_id).await?;
    }
    let receipt = store
        .send_message(
            &key(),
            workspace_id,
            project_id,
            &large.id,
            "Eligibility boundary",
        )
        .await?;
    let bytes: Vec<u8> = client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&receipt.turn_id],
        )
        .await?
        .get(0);
    let (_, _, command) = ayni_orchestrator_events::workflow::decode_command(&bytes)?;
    let reference = command
        .context_manifest
        .as_ref()
        .ok_or("missing manifest")?;
    assert_eq!(reference.record_count, 103);
    assert!(!reference.inline_complete);
    assert!(command.context.len() < 100);
    assert_eq!(
        command
            .frozen_configuration
            .as_ref()
            .map(|c| c.revision_id.as_str()),
        Some(
            store
                .get_agent_configuration(workspace_id, "")
                .await?
                .revision_id
                .as_str()
        )
    );
    assert_manifest_pages(
        store,
        workspace_id,
        project_id,
        &large.id,
        &receipt.turn_id,
        &reference.digest,
    )
    .await?;
    store
        .send_message(
            &key(),
            workspace_id,
            project_id,
            &large.id,
            "Later queued input",
        )
        .await?;
    let still = store
        .get_turn_context_inputs(
            workspace_id,
            project_id,
            &large.id,
            &receipt.turn_id,
            0,
            100,
        )
        .await?;
    assert_eq!(still.manifest.record_count, 103);
    assert!(
        still
            .records
            .iter()
            .all(|r| r.content != "Later queued input")
    );
    driver.abort();
    Ok(())
}

async fn complete_turn(
    store: &WorkflowStore,
    client: &tokio_postgres::Client,
    turn: &str,
) -> TestResult {
    use ayni_orchestrator_events::{
        EventIdentity,
        v1::{ConversationTurnReport, ConversationTurnReportKind},
        workflow::{decode_command, encode_report, sign_report, signing_key_digest},
    };
    use sha2::{Digest as _, Sha256};
    let bytes: Vec<u8> = client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&turn],
        )
        .await?
        .get(0);
    let (identity, metadata, command) = decode_command(&bytes)?;
    store.mark_outbox_stored(&identity.id, &bytes).await?;
    let mut binding = command.binding.ok_or("missing binding")?;
    binding.command_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    let seed = [73_u8; 32];
    let public = ed25519_dalek::SigningKey::from_bytes(&seed)
        .verifying_key()
        .to_bytes();
    let executor = key();
    for kind in [
        ConversationTurnReportKind::Started,
        ConversationTurnReportKind::Completed,
    ] {
        let identity = EventIdentity {
            source: format!("ayni://orchestrator/supervisors/{executor}"),
            id: key(),
        };
        let mut report = ConversationTurnReport {
            binding: Some(binding.clone()),
            kind: kind.into(),
            report_sequence: if kind == ConversationTurnReportKind::Started {
                1
            } else {
                2
            },
            executor_instance_id: executor.clone(),
            signing_public_key: if kind == ConversationTurnReportKind::Started {
                public.to_vec()
            } else {
                Vec::new()
            },
            signing_key_digest: signing_key_digest(&public),
            response_content: if kind == ConversationTurnReportKind::Completed {
                "a".repeat(16_000)
            } else {
                String::new()
            },
            failure_reason: String::new(),
            signature: Vec::new(),
        };
        sign_report(&identity, &mut report, &seed)?;
        let exact = encode_report(&identity, &metadata, &report)?;
        let decision = if kind == ConversationTurnReportKind::Started {
            store.accept_claim(&exact).await?
        } else {
            store.accept_terminal_report(&exact).await?
        };
        assert!(decision.accepted, "{}", decision.safe_reason);
    }
    Ok(())
}
fn key() -> String {
    Uuid::now_v7().to_string()
}

async fn assert_manifest_pages(
    store: &WorkflowStore,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
    turn_id: &str,
    digest: &str,
) -> TestResult {
    let mut ordinal = 0;
    let mut count = 0;
    loop {
        let page = store
            .get_turn_context_inputs(
                workspace_id,
                project_id,
                conversation_id,
                turn_id,
                ordinal,
                17,
            )
            .await?;
        assert_eq!(page.manifest.digest, digest);
        count += page.records.len();
        if page.next_after_ordinal == 0 {
            break;
        }
        ordinal = page.next_after_ordinal;
    }
    assert_eq!(count, 103);
    Ok(())
}
