use super::*;
fn conversation(store: &mut CleanStore) -> Result<v1::ProjectConversation, CleanStoreError> {
    let (repository, profile, _) = ready_profile(store)?;
    let request = task_request(store, &repository.id, &profile.id, "context-task")?
        .request
        .ok_or(CleanStoreError::CorruptState)?;
    store.create_project_conversation(
        "context-create",
        &v1::CreateProjectConversationRequest {
            planner_idle_timeout_seconds: None,
            idempotency_key: "create-context".into(),
            project_id: request.project_id,
            title: "Context archive".into(),
            repository_connection_ids: vec![repository.id],
        },
    )
}
fn message(index: usize, content: String) -> v1::ProjectMessage {
    v1::ProjectMessage {
        id: format!("context-message-{index}"),
        role: "user".into(),
        content,
        ..Default::default()
    }
}
#[test]
fn long_history_keeps_originals_searchable_and_freezes_older_relevance()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::open(":memory:", 3)?;
    let mut c = conversation(&mut store)?;
    c.messages.push(message(0,"Decision: preserve lavender repository invariant. Research https://example.test/design; open question about retries".into()));
    store.save_conversation(&mut c)?;
    for i in 1..620 {
        c.messages
            .push(message(i, format!("Recent unrelated discussion {i}")));
        store.save_conversation(&mut c)?;
    }
    assert_eq!(c.messages.len(), 16);
    c.messages
        .push(message(620, "Review lavender before implementing".into()));
    store.save_conversation(&mut c)?;
    let bundle = store.frozen_conversation_context(&c)?;
    let records = bundle["records"]
        .as_array()
        .ok_or(CleanStoreError::CorruptState)?;
    assert!(
        records
            .iter()
            .any(|r| r["id"] == "message:context-message-0")
    );
    assert!(records.len() <= 116);
    assert_eq!(bundle["complete"], false);
    assert!(bundle.to_string().len() < 110_000);
    let old = store.project_conversation_history(&v1::ListProjectConversationHistoryRequest {
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        query: "lavender repository".into(),
        limit: 10,
        ..Default::default()
    })?;
    assert_eq!(old.records.len(), 1);
    assert_eq!(old.records[0].source_id, "context-message-0");
    let frozen = old.snapshot_sequence;
    c.messages
        .push(message(621, "lavender repository new discussion".into()));
    store.save_conversation(&mut c)?;
    let unchanged =
        store.project_conversation_history(&v1::ListProjectConversationHistoryRequest {
            project_id: c.project_id.clone(),
            conversation_id: c.id.clone(),
            query: "lavender repository".into(),
            snapshot_sequence: frozen,
            ..Default::default()
        })?;
    assert_eq!(unchanged.records, old.records);
    assert!(matches!(
        store.project_conversation_context("wrong-project", &c.id),
        Err(CleanStoreError::ParentMismatch)
    ));
    let metadata = store.project_conversation_context(&c.project_id, &c.id)?;
    assert_eq!(metadata.source_watermark, 622);
    assert_eq!(
        metadata.summary.as_ref().map(|s| s.source_watermark),
        Some(622)
    );
    Ok(())
}
#[test]
fn failed_archive_and_interrupted_compaction_preserve_previous_atomic_state()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::open(":memory:", 3)?;
    let mut c = conversation(&mut store)?;
    c.messages.push(message(0, "Original decision".into()));
    store.save_conversation(&mut c)?;
    let original = c.clone();
    let summary = store.project_conversation_context(&c.project_id, &c.id)?;
    c.messages[0].content = "Conflicting replay must not replace original".into();
    assert!(matches!(
        store.save_conversation(&mut c),
        Err(CleanStoreError::StaleVersion)
    ));
    assert_eq!(store.project_conversation(&c.id)?, original);
    assert_eq!(
        store.project_conversation_context(&c.project_id, &c.id)?,
        summary
    );
    c = original.clone();
    c.messages.push(message(1, "New pending change".into()));
    store.connection.execute_batch("CREATE TRIGGER interrupt_summary BEFORE UPDATE ON project_context_summaries BEGIN SELECT RAISE(ABORT,'interrupted compaction'); END;").map_err(CleanStoreError::Database)?;
    assert!(store.save_conversation(&mut c).is_err());
    assert_eq!(store.project_conversation(&c.id)?, original);
    assert_eq!(
        store.project_conversation_context(&c.project_id, &c.id)?,
        summary
    );
    store
        .connection
        .execute_batch("DROP TRIGGER interrupt_summary")
        .map_err(CleanStoreError::Database)?;
    store.save_conversation(&mut c)?;
    assert_eq!(
        store
            .project_conversation_context(&c.project_id, &c.id)?
            .source_watermark,
        2
    );
    Ok(())
}
#[test]
fn oversized_originals_are_losslessly_chunked_and_pages_stay_bounded() -> Result<(), CleanStoreError>
{
    let mut store = CleanStore::open(":memory:", 3)?;
    let mut c = conversation(&mut store)?;
    // Plan metadata may exceed a single record; no research text may disappear.
    let original = "λ".repeat(80_000);
    c.messages.push(message(0, original.clone()));
    store.save_conversation(&mut c)?;
    let page = store.project_conversation_history(&v1::ListProjectConversationHistoryRequest {
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        limit: 100,
        ..Default::default()
    })?;
    let reconstructed = page
        .records
        .iter()
        .rev()
        .map(|r| r.content.as_str())
        .collect::<String>();
    assert_eq!(reconstructed, original);
    assert!(page.encoded_len() < 512 * 1024);
    assert!(page.records.iter().all(|r| r.content.len() <= 65_536));
    Ok(())
}

#[test]
fn original_message_metadata_survives_compaction_and_restart() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("original.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let mut c = conversation(&mut store)?;
    let original = v1::ProjectMessage {
        id: "original-with-metadata".into(),
        role: "assistant".into(),
        content: "Preserve this original research question".into(),
        run_id: "original-research-run".into(),
        created_at: Some(prost_types::Timestamp {
            seconds: 12345,
            nanos: 42,
        }),
    };
    c.messages.push(original.clone());
    store.save_conversation(&mut c)?;
    for i in 1..40 {
        c.messages.push(message(i, format!("Unrelated {i}")));
        store.save_conversation(&mut c)?;
    }
    assert!(!c.messages.iter().any(|m| m.id == original.id));
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    let bytes:Vec<u8>=store.connection.query_row("SELECT original_payload FROM project_context_records WHERE conversation_id=?1 AND source_id=?2",params![c.id,original.id],|r|r.get(0)).map_err(CleanStoreError::Database)?;
    let restored: v1::ProjectMessage = decode(&bytes)?;
    assert_eq!(restored, original);
    let before = store.project_conversation(&c.id)?;
    c = before.clone();
    let mut conflicting = original;
    conflicting.run_id = "different-run".into();
    c.messages.push(conflicting);
    assert!(matches!(
        store.save_conversation(&mut c),
        Err(CleanStoreError::StaleVersion)
    ));
    assert_eq!(store.project_conversation(&c.id)?, before);
    Ok(())
}

#[test]
fn archive_capacity_rejects_new_work_atomically_and_preserves_receipt_replay()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::open(":memory:", 3)?;
    let c = conversation(&mut store)?;
    let original = v1::SendProjectMessageRequest {
        idempotency_key: "capacity-original".into(),
        conversation_id: c.id.clone(),
        content: "Keep the original decision".into(),
    };
    let run = v1::ProjectAgentRun {
        id: "capacity-original-run".into(),
        state: "queued".into(),
        purpose: "research".into(),
        ..Default::default()
    };
    store.send_project_message("capacity-original", &original, run)?;
    store.fail_project_run("capacity-original-run", "interrupted")?;
    let before = store.project_conversation(&c.id)?;
    // Fill the real per-conversation capacity with immutable synthetic research records.
    store.connection.execute("WITH RECURSIVE seq(n) AS (SELECT 2 UNION ALL SELECT n+1 FROM seq WHERE n<100000) INSERT INTO project_context_records SELECT ?1,n,'capacity-'||n,'plan','capacity-source-'||n,'','assistant','Retained research reference',X'' FROM seq",[&c.id]).map_err(CleanStoreError::Database)?;
    super::super::project_context::archive(&store.connection, &before)?;
    let metadata = store.project_conversation_context(&c.project_id, &c.id)?;
    assert_eq!(metadata.archive_record_count, 100_000);
    let next = v1::SendProjectMessageRequest {
        idempotency_key: "capacity-next".into(),
        conversation_id: c.id.clone(),
        content: "Next question must report capacity without losing history".into(),
    };
    let run = v1::ProjectAgentRun {
        id: "capacity-next-run".into(),
        state: "queued".into(),
        purpose: "research".into(),
        ..Default::default()
    };
    let error = store
        .send_project_message("capacity-next", &next, run)
        .err()
        .ok_or(CleanStoreError::CorruptState)?;
    assert!(matches!(error, CleanStoreError::ConversationCapacity));
    assert!(error.to_string().contains("start another conversation"));
    assert_eq!(store.project_conversation(&c.id)?, before);
    assert_eq!(
        store.project_conversation_context(&c.project_id, &c.id)?,
        metadata
    );
    assert!(store.replay_conversation("capacity-next", &next)?.is_none());
    assert_eq!(
        store.replay_conversation("capacity-original", &original)?,
        Some(before)
    );
    let retained =
        store.project_conversation_history(&v1::ListProjectConversationHistoryRequest {
            project_id: c.project_id,
            conversation_id: c.id,
            query: "original decision".into(),
            ..Default::default()
        })?;
    assert_eq!(retained.records.len(), 1);
    assert_eq!(retained.records[0].content, original.content);
    Ok(())
}
