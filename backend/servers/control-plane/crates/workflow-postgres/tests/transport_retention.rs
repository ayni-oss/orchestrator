//! Real `PostgreSQL` quarantine persistence and pressure evidence. The fixture uses
//! only the explicitly supplied disposable acceptance database.

use ayni_orchestrator_events::workflow::transport::ConsumerRole;
use ayni_orchestrator_workflow_postgres::{StoreError, WorkflowStore, connect};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const FUTURE: &[u8] = br#"{"specversion":"1.0","id":"018f0f7b-6a4d-7a11-8d77-111213141516","source":"ayni://orchestrator/supervisors/018f0f7b-6a4d-7a11-8d77-010203040506","type":"build.ayni.orchestrator.conversation.turn.completed.v2","dataschema":"ayni://orchestrator/schemas/conversation-turn-report/v2","datacontenttype":"application/json","time":"2026-10-20T08:00:00Z","traceparent":"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01","workspaceid":"workspace:018f0f7b-6a4d-7a11-8d77-111213141516","projectid":"project:018f0f7b-6a4d-7a11-8d77-111213141516","conversationid":"conversation:018f0f7b-6a4d-7a11-8d77-111213141516","turnid":"turn:018f0f7b-6a4d-7a11-8d77-111213141516","assignmentid":"018f0f7b-6a4d-7a11-8d77-111213141516","causationid":"018f0f7b-6a4d-7a11-8d77-111213141516","correlationid":"conversation:018f0f7b-6a4d-7a11-8d77-111213141516","executorgeneration":"1","subject":"conversations/conversation:018f0f7b-6a4d-7a11-8d77-111213141516/turns/turn:018f0f7b-6a4d-7a11-8d77-111213141516","data":{}}"#;

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL acceptance fixture"]
async fn quarantine_preserves_future_bytes_and_backpressures_without_eviction() -> TestResult {
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let (client, driver) = connect(&url).await?;
    let store = WorkflowStore::new(url.as_str());
    assert_eq!(
        store.transport_health().await?.records,
        0,
        "fixture must start empty"
    );
    store
        .quarantine_transport(
            ConsumerRole::ApplicationReport,
            b"private malformed fixture",
        )
        .await?;
    store
        .quarantine_transport(
            ConsumerRole::ApplicationReport,
            b"private malformed fixture",
        )
        .await?;
    assert_eq!(store.transport_health().await?.records, 1);
    assert!(store.quarantine_replay_batch().await?.is_empty());
    store
        .quarantine_transport(ConsumerRole::ApplicationReport, FUTURE)
        .await?;
    store
        .quarantine_transport(ConsumerRole::ApplicationReport, FUTURE)
        .await?;
    let mut conflicting = FUTURE.to_vec();
    conflicting.push(b' ');
    store
        .quarantine_transport(ConsumerRole::ApplicationReport, &conflicting)
        .await?;
    let retained = store.quarantine_replay_batch().await?;
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].exact_bytes, FUTURE);
    // Simulated retention time never expires unsupported exact content.
    client.execute("UPDATE workflow_transport_quarantine SET recorded_at=clock_timestamp()-interval '9 days'",&[]).await?;
    assert_eq!(store.cleanup_transport_quarantine().await?, 2);
    assert_eq!(store.transport_health().await?.unsupported, 1);
    // Capacity fixtures may directly populate diagnostic records; no domain data is changed.
    client.execute("INSERT INTO workflow_transport_quarantine(consumer_role,event_source,event_id,event_digest,reason) SELECT 'application_report','capacity_fixture',i::text,'sha256:'||repeat('0',64),'malformed' FROM generate_series(1,1023) i",&[]).await?;
    assert!(store.transport_health().await?.full);
    assert!(matches!(
        store
            .quarantine_transport(ConsumerRole::ApplicationReport, b"over capacity")
            .await,
        Err(StoreError::Capacity("application_report_quarantine"))
    ));
    // A duplicate needs no additional quota and retains its original exact bytes.
    store
        .quarantine_transport(ConsumerRole::ApplicationReport, FUTURE)
        .await?;
    assert_eq!(
        store.quarantine_replay_batch().await?[0].exact_bytes,
        FUTURE
    );
    client
        .execute(
            "DELETE FROM workflow_transport_quarantine WHERE event_source='capacity_fixture'",
            &[],
        )
        .await?;
    store.resolve_quarantine(&retained[0]).await?;
    assert_eq!(store.transport_health().await?.records, 0);
    driver.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL acceptance fixture"]
async fn terminal_before_claim_reconciles_once_from_retained_exact_bytes() -> TestResult {
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let store = WorkflowStore::new(url.as_str());
    let key = uuid::Uuid::now_v7().to_string();
    let workspace = store
        .create_workspace(&format!("ordered-workspace-{key}"), "Ordered fixture")
        .await?
        .resource
        .id;
    let project = store
        .create_project(
            &format!("ordered-project-{key}"),
            &workspace,
            "Ordered fixture",
            "",
        )
        .await?
        .resource
        .id;
    let conversation = store
        .create_conversation(
            &format!("ordered-conversation-{key}"),
            &workspace,
            &project,
            "Ordered fixture",
        )
        .await?
        .resource
        .id;
    let receipt = store
        .send_message(
            &format!("ordered-message-{key}"),
            &workspace,
            &project,
            &conversation,
            "ordered fixture",
        )
        .await?;
    let command = store
        .pending_outbox(1000)
        .await?
        .into_iter()
        .find(|record| record.turn_id == receipt.turn_id)
        .ok_or("fixture command absent")?;
    let reports = ordered_reports(&command.event_bytes)?;
    assert!(store.retain_ordered_report(&reports[1]).await?);
    assert!(store.retain_ordered_report(&reports[1]).await?);
    assert_eq!(store.reconcile_pending_reports().await?, 0);
    let before = store
        .get_snapshot(&workspace, &project, &conversation)
        .await?;
    assert_eq!(before.nonterminal_turns.len(), 1);
    assert!(before.recent_terminal_turns.is_empty());
    // A new store has no process memory of the prior staged delivery.
    drop(store);
    let store = WorkflowStore::new(url.as_str());
    assert!(store.accept_claim(&reports[0]).await?.accepted);
    assert_eq!(store.reconcile_pending_reports().await?, 1);
    assert_eq!(store.reconcile_pending_reports().await?, 0);
    let first = store.accept_terminal_report(&reports[1]).await?;
    assert!(first.accepted);
    assert_eq!(first, store.accept_terminal_report(&reports[1]).await?);
    let after = store
        .get_snapshot(&workspace, &project, &conversation)
        .await?;
    assert!(after.nonterminal_turns.is_empty());
    assert_eq!(after.recent_terminal_turns.len(), 1);
    assert_eq!(
        after
            .recent_messages
            .iter()
            .filter(|message| message.content == "ordered response")
            .count(),
        1
    );
    let (client, driver) = connect(&url).await?;
    let pending: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_pending_reports WHERE turn_id=$1",
            &[&receipt.turn_id],
        )
        .await?
        .get(0);
    assert_eq!(pending, 0);
    prove_detached_rejections(&store, &reports, &workspace, &project, &conversation).await?;
    driver.abort();
    Ok(())
}

fn ordered_reports(
    command_bytes: &[u8],
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    use ayni_orchestrator_events::{
        EventIdentity,
        v1::{ConversationTurnReport, ConversationTurnReportKind},
        workflow::{decode_command, encode_report, sign_report, signing_key_digest},
    };
    use ed25519_dalek::SigningKey;
    use sha2::{Digest as _, Sha256};
    let (_, metadata, payload) = decode_command(command_bytes)?;
    let mut binding = payload.binding.ok_or("fixture command binding absent")?;
    binding.command_digest = format!("sha256:{:x}", Sha256::digest(command_bytes));
    let seed = [7; 32];
    let public_key = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let executor = uuid::Uuid::now_v7().to_string();
    let mut reports = Vec::new();
    for kind in [
        ConversationTurnReportKind::Started,
        ConversationTurnReportKind::Completed,
    ] {
        let identity = EventIdentity {
            source: format!("ayni://orchestrator/supervisors/{executor}"),
            id: uuid::Uuid::now_v7().to_string(),
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
                public_key.to_vec()
            } else {
                Vec::new()
            },
            signing_key_digest: signing_key_digest(&public_key),
            response_content: if kind == ConversationTurnReportKind::Completed {
                "ordered response".into()
            } else {
                String::new()
            },
            ..Default::default()
        };
        sign_report(&identity, &mut report, &seed)?;
        reports.push(encode_report(&identity, &metadata, &report)?);
    }
    Ok(reports)
}

async fn prove_detached_rejections(
    store: &WorkflowStore,
    reports: &[Vec<u8>],
    workspace: &str,
    project: &str,
    conversation: &str,
) -> TestResult {
    use ayni_orchestrator_events::workflow::{decode_report, encode_report, sign_report};
    let before = store.get_snapshot(workspace, project, conversation).await?;
    let (identity, metadata, mut report) = decode_report(&reports[1])?;
    report.response_content = "different exact response".into();
    sign_report(&identity, &mut report, &[7; 32])?;
    let changed = encode_report(&identity, &metadata, &report)?;
    let conflict = store.accept_terminal_report(&changed).await?;
    assert!(!conflict.accepted);
    assert_eq!(conflict.safe_reason, "event_conflict");
    assert_eq!(conflict, store.accept_terminal_report(&changed).await?);
    let (mut identity, metadata, mut report) = decode_report(&reports[0])?;
    identity.id = uuid::Uuid::now_v7().to_string();
    report
        .binding
        .as_mut()
        .ok_or("fixture binding absent")?
        .workspace_id = format!("workspace:{}", uuid::Uuid::now_v7());
    sign_report(&identity, &mut report, &[7; 32])?;
    let wrong = encode_report(&identity, &metadata, &report)?;
    let rejected = store.accept_claim(&wrong).await?;
    assert!(!rejected.accepted);
    assert_eq!(rejected.safe_reason, "wrong_scope");
    assert_eq!(rejected, store.accept_claim(&wrong).await?);
    let after = store.get_snapshot(workspace, project, conversation).await?;
    assert_eq!(before.conversation_revision, after.conversation_revision);
    assert_eq!(before.recent_messages, after.recent_messages);
    assert_eq!(before.recent_terminal_turns, after.recent_terminal_turns);
    let outbox = store.pending_outbox(1000).await?;
    assert!(
        outbox
            .iter()
            .any(|event| event.id == conflict.decision_event_id)
    );
    assert!(
        outbox
            .iter()
            .any(|event| event.id == rejected.decision_event_id)
    );
    let detached = outbox
        .iter()
        .find(|event| event.id == rejected.decision_event_id)
        .ok_or("detached decision absent")?;
    prove_detached_failure_preserves_copy_age(store, detached).await?;
    prove_unmaterialized_report_rejected(store, &reports[0], workspace, project, conversation)
        .await?;
    Ok(())
}

async fn prove_unmaterialized_report_rejected(
    store: &WorkflowStore,
    claim: &[u8],
    workspace: &str,
    project: &str,
    conversation: &str,
) -> TestResult {
    use ayni_orchestrator_events::workflow::{decode_report, encode_report, sign_report};
    let key = uuid::Uuid::now_v7().to_string();
    store
        .send_message(
            &format!("unmaterialized-head-{key}"),
            workspace,
            project,
            conversation,
            "head",
        )
        .await?;
    let queued = store
        .send_message(
            &format!("unmaterialized-tail-{key}"),
            workspace,
            project,
            conversation,
            "waiting",
        )
        .await?;
    let (mut identity, metadata, mut report) = decode_report(claim)?;
    identity.id = uuid::Uuid::now_v7().to_string();
    report
        .binding
        .as_mut()
        .ok_or("fixture binding absent")?
        .turn_id = queued.turn_id;
    sign_report(&identity, &mut report, &[7; 32])?;
    let bytes = encode_report(&identity, &metadata, &report)?;
    let rejection = store.accept_claim(&bytes).await?;
    assert!(!rejection.accepted);
    assert_eq!(rejection.safe_reason, "stale_generation");
    let snapshot = store.get_snapshot(workspace, project, conversation).await?;
    assert_eq!(snapshot.nonterminal_turns.len(), 2);
    Ok(())
}

struct DeliveryFixture {
    store: WorkflowStore,
    workspace: String,
    project: String,
    conversation: String,
    key: String,
    receipt: ayni_orchestrator_workflow_postgres::model::CommandReceipt,
    command: ayni_orchestrator_workflow_postgres::model::OutboxRecord,
}

async fn delivery_fixture() -> Result<DeliveryFixture, Box<dyn std::error::Error + Send + Sync>> {
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let store = WorkflowStore::new(url.as_str());
    let key = uuid::Uuid::now_v7().to_string();
    let workspace = store
        .create_workspace(&format!("failure-workspace-{key}"), "Delivery fixture")
        .await?
        .resource
        .id;
    let project = store
        .create_project(
            &format!("failure-project-{key}"),
            &workspace,
            "Delivery fixture",
            "",
        )
        .await?
        .resource
        .id;
    let conversation = store
        .create_conversation(
            &format!("failure-conversation-{key}"),
            &workspace,
            &project,
            "Delivery fixture",
        )
        .await?
        .resource
        .id;
    let receipt = store
        .send_message(
            &key,
            &workspace,
            &project,
            &conversation,
            "delivery fixture",
        )
        .await?;
    let command = store
        .pending_outbox(1000)
        .await?
        .into_iter()
        .find(|event| event.turn_id == receipt.turn_id)
        .ok_or("fixture command absent")?;
    Ok(DeliveryFixture {
        store,
        workspace,
        project,
        conversation,
        key,
        receipt,
        command,
    })
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL acceptance fixture"]
async fn delivery_failure_and_recovery_preserve_original_receipt_and_frozen_binding() -> TestResult
{
    let fixture = delivery_fixture().await?;
    let failed_revision = prove_delivery_failure(&fixture).await?;
    prove_delivery_recovery(&fixture, failed_revision).await?;
    prove_claimed_failure_no_regression(&fixture).await
}

async fn prove_delivery_failure(
    fixture: &DeliveryFixture,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    use ayni_orchestrator_workflow_postgres::model::DispatchState;
    let DeliveryFixture {
        store,
        workspace,
        project,
        conversation,
        key,
        receipt,
        command,
    } = fixture;
    let before = store.get_snapshot(workspace, project, conversation).await?;
    store
        .mark_outbox_failed(&command.id, &command.event_bytes, "broker_unavailable")
        .await?;
    let failed = store.get_snapshot(workspace, project, conversation).await?;
    assert_eq!(
        failed.conversation_revision,
        before.conversation_revision + 1
    );
    assert_eq!(
        failed.nonterminal_turns[0].dispatch_state,
        DispatchState::Degraded
    );
    assert_eq!(
        failed.nonterminal_turns[0].assignment_id,
        before.nonterminal_turns[0].assignment_id
    );
    assert_eq!(
        failed.nonterminal_turns[0].executor_generation,
        before.nonterminal_turns[0].executor_generation
    );
    let activity = failed
        .current_activity
        .as_ref()
        .ok_or("delivery activity absent")?;
    assert_eq!(activity.stage, "dispatch");
    assert_eq!(activity.waiting_on, "delivery");
    assert_eq!(activity.safe_reason, "broker_unavailable");
    store
        .mark_outbox_failed(&command.id, &command.event_bytes, "broker_unavailable")
        .await?;
    assert_eq!(
        store
            .get_snapshot(workspace, project, conversation)
            .await?
            .conversation_revision,
        failed.conversation_revision
    );
    assert_eq!(
        &store
            .send_message(key, workspace, project, conversation, "delivery fixture")
            .await?,
        receipt
    );
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let (client, driver) = connect(&url).await?;
    let row=client.query_one("SELECT event_bytes,last_safe_reason,attempt_count,next_attempt_at>updated_at FROM workflow_outbox WHERE id=$1",&[&command.id]).await?;
    assert_eq!(row.get::<_, Vec<u8>>(0), command.event_bytes);
    assert_eq!(row.get::<_, String>(1), "broker_unavailable");
    assert_eq!(row.get::<_, i64>(2), 2);
    assert!(row.get::<_, bool>(3));
    driver.abort();
    Ok(failed.conversation_revision)
}

async fn prove_delivery_recovery(fixture: &DeliveryFixture, failed_revision: u64) -> TestResult {
    use ayni_orchestrator_workflow_postgres::model::DispatchState;
    let DeliveryFixture {
        store,
        workspace,
        project,
        conversation,
        command,
        ..
    } = fixture;
    store
        .mark_outbox_publication(&command.id, &command.event_bytes, 91)
        .await?;
    let recovered = store.get_snapshot(workspace, project, conversation).await?;
    assert_eq!(
        recovered.nonterminal_turns[0].dispatch_state,
        DispatchState::Stored
    );
    assert_eq!(recovered.conversation_revision, failed_revision + 1);
    assert_eq!(
        recovered
            .current_activity
            .as_ref()
            .ok_or("recovery activity absent")?
            .safe_reason,
        ""
    );
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let (client, driver) = connect(&url).await?;
    let stored_at: String = client
        .query_one(
            "SELECT updated_at::text FROM workflow_outbox WHERE id=$1",
            &[&command.id],
        )
        .await?
        .get(0);
    store
        .mark_outbox_failed(&command.id, &command.event_bytes, "broker_capacity")
        .await?;
    let failed_retry_at: String = client
        .query_one(
            "SELECT updated_at::text FROM workflow_outbox WHERE id=$1",
            &[&command.id],
        )
        .await?
        .get(0);
    assert_eq!(
        stored_at, failed_retry_at,
        "failed retry must not refresh broker-copy age"
    );
    driver.abort();
    Ok(())
}

async fn prove_claimed_failure_no_regression(fixture: &DeliveryFixture) -> TestResult {
    use ayni_orchestrator_workflow_postgres::model::{DispatchState, ExecutionState};
    let DeliveryFixture {
        store,
        workspace,
        project,
        conversation,
        key,
        receipt,
        command,
    } = fixture;
    let reports = ordered_reports(&command.event_bytes)?;
    assert!(store.accept_claim(&reports[0]).await?.accepted);
    let claimed = store.get_snapshot(workspace, project, conversation).await?;
    store
        .mark_outbox_failed(&command.id, &command.event_bytes, "broker_capacity")
        .await?;
    let after = store.get_snapshot(workspace, project, conversation).await?;
    assert_eq!(after.conversation_revision, claimed.conversation_revision);
    assert_eq!(
        after.nonterminal_turns[0].dispatch_state,
        DispatchState::Transferred
    );
    assert_eq!(
        after.nonterminal_turns[0].execution_state,
        ExecutionState::Processing
    );
    assert_eq!(
        &store
            .send_message(key, workspace, project, conversation, "delivery fixture")
            .await?,
        receipt
    );
    Ok(())
}

async fn prove_detached_failure_preserves_copy_age(
    store: &WorkflowStore,
    event: &ayni_orchestrator_workflow_postgres::model::OutboxRecord,
) -> TestResult {
    store
        .mark_outbox_publication(&event.id, &event.event_bytes, 99)
        .await?;
    let url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let (client, driver) = connect(&url).await?;
    let prior: String = client
        .query_one(
            "SELECT updated_at::text FROM workflow_transport_rejections WHERE decision_event_id=$1",
            &[&event.id],
        )
        .await?
        .get(0);
    store
        .mark_outbox_failed(&event.id, &event.event_bytes, "broker_unavailable")
        .await?;
    let after=client.query_one("SELECT updated_at::text,safe_reason FROM workflow_transport_rejections WHERE decision_event_id=$1",&[&event.id]).await?;
    assert_eq!(prior, after.get::<_, String>(0));
    assert_eq!(after.get::<_, String>(1), "wrong_scope");
    driver.abort();
    Ok(())
}
