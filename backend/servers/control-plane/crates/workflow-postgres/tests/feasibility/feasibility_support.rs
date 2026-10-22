use super::*;

pub async fn prove_capacity_race(
    client: &Client,
    store: &WorkflowStore,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
) -> TestResult {
    for index in 17..96 {
        store
            .send_message(
                &format!("capacity-{index}"),
                workspace_id,
                project_id,
                conversation_id,
                "queued",
            )
            .await?;
    }
    let callers = 8;
    let barrier = Arc::new(Barrier::new(callers));
    let mut sends = Vec::new();
    for index in 0..callers {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        let workspace_id = workspace_id.to_owned();
        let project_id = project_id.to_owned();
        let conversation_id = conversation_id.to_owned();
        sends.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .send_message(
                    &format!("capacity-race-{index}"),
                    &workspace_id,
                    &project_id,
                    &conversation_id,
                    "queued",
                )
                .await
        }));
    }
    let mut admitted = 0;
    let mut rejected = 0;
    for send in sends {
        match send.await? {
            Ok(_) => admitted += 1,
            Err(StoreError::Capacity("turn_queue")) => rejected += 1,
            Err(error) => return Err(Box::new(error)),
        }
    }
    assert_eq!(admitted, 4);
    assert_eq!(rejected, 4);
    let retained: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_turns WHERE conversation_id = $1",
            &[&conversation_id],
        )
        .await?
        .get(0);
    assert_eq!(retained, 100);
    Ok(())
}

pub async fn prove_future_version_rejected(client: &Client, database_url: &str) -> TestResult {
    let future = CURRENT_SCHEMA_VERSION + 1;
    client.execute("INSERT INTO workflow_schema_migrations (version, description, checksum) VALUES ($1, 'future', 'sha256:future')", &[&future]).await?;
    assert!(matches!(
        validate(database_url).await,
        Err(MigrationError::FutureVersion(version)) if version == future
    ));
    client
        .execute(
            "DELETE FROM workflow_schema_migrations WHERE version = $1",
            &[&future],
        )
        .await?;
    Ok(())
}

pub async fn prove_stored_command_retry(
    client: &Client,
    store: &WorkflowStore,
    record: &ayni_orchestrator_workflow_postgres::model::OutboxRecord,
) -> TestResult {
    store
        .mark_outbox_publication(&record.id, &record.event_bytes, 11)
        .await?;
    assert!(
        !store
            .pending_outbox(1000)
            .await?
            .iter()
            .any(|item| item.id == record.id)
    );
    // Fault clock: the broker copy may have expired; only scheduling timestamps
    // change, while the original authority and exact envelope remain frozen.
    client.execute("UPDATE workflow_outbox SET next_attempt_at=clock_timestamp()-interval '8 days' WHERE id=$1", &[&record.id]).await?;
    let recovered = store
        .pending_outbox(1000)
        .await?
        .into_iter()
        .find(|item| item.id == record.id)
        .ok_or("stored unclaimed command did not become retryable")?;
    assert_eq!(recovered.event_bytes, record.event_bytes);
    let reopened = WorkflowStore::new(std::env::var(RUNTIME_DATABASE_URL_ENV)?);
    let recovered = reopened
        .pending_outbox(1000)
        .await?
        .into_iter()
        .find(|item| item.id == record.id)
        .ok_or("restart lost stored command retry")?;
    assert_eq!(recovered.event_bytes, record.event_bytes);
    Ok(())
}

pub async fn prove_targeted_ack_release(
    store: &WorkflowStore,
    claim: &ayni_orchestrator_workflow_postgres::model::ReportDecisionReceipt,
    terminal: &ayni_orchestrator_workflow_postgres::model::ReportDecisionReceipt,
) -> TestResult {
    for receipt in [claim, terminal] {
        let record = store
            .pending_outbox(1000)
            .await?
            .into_iter()
            .find(|record| record.id == receipt.decision_event_id)
            .ok_or("retained report decision is absent")?;
        store
            .mark_outbox_publication(&record.id, &record.event_bytes, 42)
            .await?;
        assert!(
            store
                .acknowledge_decision_delivery(&record.id, 42, 41)
                .await
                .is_err()
        );
        assert!(
            store
                .decision_acknowledgements()
                .await?
                .iter()
                .any(|ack| ack.event_id == record.id)
        );
        // A stale observation cannot release a newer publication.
        store
            .mark_outbox_publication(&record.id, &record.event_bytes, 43)
            .await?;
        store
            .acknowledge_decision_delivery(&record.id, 42, 42)
            .await?;
        assert!(
            store
                .decision_acknowledgements()
                .await?
                .iter()
                .any(|ack| ack.event_id == record.id)
        );
        store
            .acknowledge_decision_delivery(&record.id, 43, 43)
            .await?;
        assert!(
            !store
                .decision_acknowledgements()
                .await?
                .iter()
                .any(|ack| ack.event_id == record.id)
        );
    }
    Ok(())
}

pub async fn prove_decision_capacity_rollbacks(
    client: &Client,
    store: &WorkflowStore,
    claim: &[u8],
) -> TestResult {
    // Capacity fixtures may allocate synthetic retained obligations directly.
    // Preserve scope/evidence columns while filling only the decision lane.
    client.execute("INSERT INTO workflow_report_receipts \
        (report_source,report_id,report_digest,exact_report_bytes,workspace_id,project_id,conversation_id,turn_id,assignment_id,executor_generation,report_sequence,executor_instance_id,decision,safe_reason,decision_event_id,decision_bytes) \
        SELECT 'capacity-fixture',i::text,r.report_digest,r.exact_report_bytes,r.workspace_id,r.project_id,r.conversation_id,r.turn_id,r.assignment_id,r.executor_generation,r.report_sequence,r.executor_instance_id,'rejected','claim_lost','capacity-decision-'||i,r.decision_bytes \
        FROM (SELECT * FROM workflow_report_receipts LIMIT 1) r CROSS JOIN generate_series(1,2731) i",&[]).await?;
    client.execute("INSERT INTO workflow_outbox(id,workspace_id,project_id,conversation_id,turn_id,subject,event_bytes) \
        SELECT decision_event_id,workspace_id,project_id,conversation_id,turn_id,'orchestrator.workflow.accepted.conversation.v1.fixture',decision_bytes \
        FROM workflow_report_receipts WHERE report_source='capacity-fixture'",&[]).await?;
    let (mut identity, metadata, payload) = decode_report(claim)?;
    identity.id = Uuid::now_v7().to_string();
    let claim = encode_report(&identity, &metadata, &payload)?;
    assert!(matches!(
        store.accept_claim(&claim).await,
        Err(StoreError::Capacity("decision_lane_reservation"))
    ));
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_report_receipts WHERE report_id=$1",
            &[&identity.id],
        )
        .await?
        .get(0);
    assert_eq!(count, 0, "capacity rejection committed a report receipt");
    let binding = payload.binding.ok_or("missing claim binding")?;
    assert!(matches!(
        store
            .send_message(
                "capacity-no-false-receipt",
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                "bounded"
            )
            .await,
        Err(StoreError::Capacity("decision_lane_reservation"))
    ));
    let count:i64=client.query_one("SELECT count(*) FROM workflow_command_receipts WHERE idempotency_key='capacity-no-false-receipt'",&[]).await?.get(0);
    assert_eq!(
        count, 0,
        "capacity rejection committed an admission receipt"
    );
    client
        .execute(
            "DELETE FROM workflow_outbox WHERE id LIKE 'capacity-decision-%'",
            &[],
        )
        .await?;
    client
        .execute(
            "DELETE FROM workflow_report_receipts WHERE report_source='capacity-fixture'",
            &[],
        )
        .await?;
    Ok(())
}

pub async fn prove_terminal_projection_and_fifo(
    client: &Client,
    store: &WorkflowStore,
    binding: &ConversationTurnBinding,
    second_turn_id: &str,
    command_event_id: &str,
) -> TestResult {
    let snapshot = store
        .get_snapshot(
            &binding.workspace_id,
            &binding.project_id,
            &binding.conversation_id,
        )
        .await?;
    assert_eq!(snapshot.nonterminal_turns.len(), 2);
    assert_eq!(snapshot.recent_terminal_turns.len(), 1);
    assert!(
        snapshot
            .recent_messages
            .iter()
            .any(|message| message.content == "scripted response")
    );
    let report_count: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_report_receipts WHERE turn_id = $1",
            &[&binding.turn_id],
        )
        .await?
        .get(0);
    assert_eq!(report_count, 5);

    let promoted = store
        .pending_outbox(1_000)
        .await?
        .into_iter()
        .filter_map(|record| decode_command(&record.event_bytes).ok())
        .find(|(_, _, command)| {
            command
                .binding
                .as_ref()
                .is_some_and(|candidate| candidate.turn_id == second_turn_id)
        })
        .ok_or("promoted command is absent")?;
    let promoted_command = promoted.2;
    assert_eq!(promoted_command.input_content, "second scripted input");
    let context = promoted_command
        .context
        .iter()
        .map(|record| record.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        context,
        [
            "first scripted input",
            "scripted response",
            "second scripted input"
        ]
    );
    assert!(!context.contains(&"third queued input"));
    assert_eq!(binding.command_event_id, command_event_id);
    Ok(())
}

pub fn signed_report(
    binding: &ConversationTurnBinding,
    metadata: &WorkflowEventMetadata,
    command_bytes: &[u8],
    executor: &str,
    seed: &[u8; 32],
    kind: ConversationTurnReportKind,
    response: &str,
) -> TestResult<Vec<u8>> {
    let identity = EventIdentity {
        source: format!("ayni://orchestrator/supervisors/{executor}"),
        id: Uuid::now_v7().to_string(),
    };
    let public_key = SigningKey::from_bytes(seed).verifying_key().to_bytes();
    let mut report_binding = binding.clone();
    report_binding.command_digest = format!("sha256:{:x}", Sha256::digest(command_bytes));
    let mut report = ConversationTurnReport {
        binding: Some(report_binding),
        kind: kind.into(),
        report_sequence: if kind == ConversationTurnReportKind::Started {
            1
        } else {
            2
        },
        executor_instance_id: executor.to_owned(),
        signing_public_key: if kind == ConversationTurnReportKind::Started {
            public_key.to_vec()
        } else {
            Vec::new()
        },
        signing_key_digest: signing_key_digest(&public_key),
        response_content: response.to_owned(),
        failure_reason: String::new(),
        signature: Vec::new(),
        provider_call: None,
        plan_proposal: None,
    };
    sign_report(&identity, &mut report, seed)?;
    Ok(encode_report(&identity, metadata, &report)?)
}
