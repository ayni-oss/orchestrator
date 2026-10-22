use ayni_orchestrator_events::{
    EventIdentity,
    v1::{ConversationTurnBinding, ConversationTurnReport, ConversationTurnReportKind},
    workflow::{
        WorkflowEventMetadata, decode_command, encode_report, sign_report, signing_key_digest,
    },
};
use ayni_orchestrator_workflow_postgres::{StoreError, WorkflowStore};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
#[allow(
    clippy::too_many_lines,
    reason = "one durable lifecycle scenario preserves ordering across expiry and replacement"
)]
async fn session_expiry_replacement_and_stale_reports_preserve_frozen_turns() -> TestResult {
    let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    ayni_orchestrator_workflow_postgres::migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(url.clone());
    let (client, driver) = ayni_orchestrator_workflow_postgres::connect(&url).await?;
    let workspace = store
        .create_workspace(&key(), "Session lifecycle")
        .await?
        .resource;
    let project = store
        .create_project(&key(), &workspace.id, "Sessions", "failure recovery")
        .await?
        .resource;
    let conversation = store
        .create_conversation(&key(), &workspace.id, &project.id, "Lifecycle")
        .await?
        .resource;
    let first = store
        .send_message(
            &key(),
            &workspace.id,
            &project.id,
            &conversation.id,
            "First generation",
        )
        .await?;
    let frozen = store
        .get_turn_inputs(&workspace.id, &project.id, &conversation.id, &first.turn_id)
        .await?;
    let initial = store
        .conversation_sessions(&workspace.id, &project.id, &conversation.id)
        .await?
        .remove(0);
    assert_eq!(initial.generation, 1);
    assert!(!initial.can_replace);
    let old = complete(&store, &client, &first.turn_id).await?;
    assert_replace_availability(&store, &workspace.id, &project.id, &conversation.id, false)
        .await?;
    drain_decisions(&store, &client, &conversation.id).await?;
    assert_replace_availability(&store, &workspace.id, &project.id, &conversation.id, true).await?;
    client.execute("UPDATE workflow_sessions SET last_active_at=clock_timestamp()-interval '2 days' WHERE conversation_id=$1",&[&conversation.id]).await?;
    let expired = store.expire_idle_sessions().await?;
    assert!(expired.iter().any(|session| session.id == initial.id
        && session.generation == 1
        && session.state == "expired"));
    let mut proposed = frozen.configuration.clone();
    proposed.instructions = "Instructions for later turns".into();
    proposed.revision_id.clear();
    let changed = store
        .update_agent_configuration(&key(), &frozen.configuration.revision_id, &proposed)
        .await?;
    let second = store
        .send_message(
            &key(),
            &workspace.id,
            &project.id,
            &conversation.id,
            "Wake after expiry",
        )
        .await?;
    let current = store
        .conversation_sessions(&workspace.id, &project.id, &conversation.id)
        .await?
        .remove(0);
    assert_eq!(current.id, initial.id);
    assert_eq!(current.generation, 2);
    let second_inputs = store
        .get_turn_inputs(
            &workspace.id,
            &project.id,
            &conversation.id,
            &second.turn_id,
        )
        .await?;
    assert_eq!(second_inputs.configuration.revision_id, changed.revision_id);
    complete(&store, &client, &second.turn_id).await?;
    let late = report(
        &old.binding,
        &old.metadata,
        &old.executor,
        ConversationTurnReportKind::Completed,
    )?;
    let rejected = store.accept_terminal_report(&late).await?;
    assert!(!rejected.accepted);
    assert_eq!(rejected.safe_reason, "stale_generation");
    drain_decisions(&store, &client, &conversation.id).await?;
    replace_and_wake(
        &store,
        &client,
        &workspace.id,
        &project.id,
        &conversation.id,
        &initial.id,
    )
    .await?;
    assert_eq!(
        store
            .get_turn_inputs(&workspace.id, &project.id, &conversation.id, &first.turn_id)
            .await?,
        frozen
    );
    driver.abort();
    Ok(())
}

async fn assert_replace_availability(
    store: &WorkflowStore,
    workspace: &str,
    project: &str,
    conversation: &str,
    expected: bool,
) -> TestResult {
    let sessions = store
        .conversation_sessions(workspace, project, conversation)
        .await?;
    assert_eq!(sessions[0].can_replace, expected);
    Ok(())
}

async fn replace_and_wake(
    store: &WorkflowStore,
    client: &tokio_postgres::Client,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
    session_id: &str,
) -> TestResult {
    let replacement_key = key();
    let replaced = store
        .replace_idle_session(
            workspace_id,
            project_id,
            conversation_id,
            2,
            &replacement_key,
        )
        .await?;
    assert_eq!(replaced.state, "replaced");
    let replay = store
        .replace_idle_session(
            workspace_id,
            project_id,
            conversation_id,
            2,
            &replacement_key,
        )
        .await?;
    assert_eq!(
        (replay.id, replay.generation, replay.state),
        (replaced.id, 2, "replaced".into())
    );
    assert!(matches!(
        store
            .replace_idle_session(
                workspace_id,
                project_id,
                conversation_id,
                1,
                &replacement_key
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    let third = store
        .send_message(
            &key(),
            workspace_id,
            project_id,
            conversation_id,
            "After replacement",
        )
        .await?;
    let latest = store
        .conversation_sessions(workspace_id, project_id, conversation_id)
        .await?
        .remove(0);
    assert_eq!(latest.generation, 3);
    assert_eq!(latest.id, session_id);
    let bytes: Vec<u8> = client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&third.turn_id],
        )
        .await?
        .get(0);
    let (_, _, command) = decode_command(&bytes)?;
    let binding = command.binding.ok_or("missing binding")?;
    assert_eq!(binding.executor_generation, 3);
    assert_eq!(command.session.ok_or("missing session")?.generation, 3);
    Ok(())
}

struct Completed {
    binding: ConversationTurnBinding,
    metadata: WorkflowEventMetadata,
    executor: String,
}

async fn complete(
    store: &WorkflowStore,
    client: &tokio_postgres::Client,
    turn: &str,
) -> Result<Completed, Box<dyn Error + Send + Sync>> {
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
    let executor = key();
    let claim = report(
        &binding,
        &metadata,
        &executor,
        ConversationTurnReportKind::Started,
    )?;
    assert!(store.accept_claim(&claim).await?.accepted);
    let completed = report(
        &binding,
        &metadata,
        &executor,
        ConversationTurnReportKind::Completed,
    )?;
    assert!(store.accept_terminal_report(&completed).await?.accepted);
    Ok(Completed {
        binding,
        metadata,
        executor,
    })
}

fn report(
    binding: &ConversationTurnBinding,
    metadata: &WorkflowEventMetadata,
    executor: &str,
    kind: ConversationTurnReportKind,
) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let seed = [83_u8; 32];
    let public = ed25519_dalek::SigningKey::from_bytes(&seed)
        .verifying_key()
        .to_bytes();
    let identity = EventIdentity {
        source: format!("ayni://orchestrator/supervisors/{executor}"),
        id: key(),
    };
    let started = kind == ConversationTurnReportKind::Started;
    let mut report = ConversationTurnReport {
        binding: Some(binding.clone()),
        kind: kind.into(),
        report_sequence: if started { 1 } else { 2 },
        executor_instance_id: executor.into(),
        signing_public_key: if started { public.to_vec() } else { Vec::new() },
        signing_key_digest: signing_key_digest(&public),
        response_content: if started {
            String::new()
        } else {
            "Retained session result".into()
        },
        failure_reason: String::new(),
        signature: Vec::new(),
        provider_call: None,
        plan_proposal: None,
    };
    sign_report(&identity, &mut report, &seed)?;
    Ok(encode_report(&identity, metadata, &report)?)
}

async fn drain_decisions(
    store: &WorkflowStore,
    client: &tokio_postgres::Client,
    conversation: &str,
) -> TestResult {
    let rows=client.query("SELECT id,event_bytes FROM workflow_outbox WHERE conversation_id=$1 AND state<>'transferred'",&[&conversation]).await?;
    for row in rows {
        let id: String = row.get(0);
        let bytes: Vec<u8> = row.get(1);
        store.mark_outbox_publication(&id, &bytes, 1).await?;
        store.acknowledge_decision_delivery(&id, 1, 1).await?;
    }
    Ok(())
}

fn key() -> String {
    Uuid::now_v7().to_string()
}
