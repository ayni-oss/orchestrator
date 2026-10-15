//! Export synthetic retained state for the Docker Desktop API/restart gate.
use super::*;

#[test]
#[ignore = "exports synthetic SQLite context fixtures for disposable local Kubernetes validation"]
fn export_project_context_fixture() -> Result<(), CleanStoreError> {
    let path =
        std::env::var("AYNI_CONTEXT_FIXTURE_PATH").map_err(|_| CleanStoreError::CorruptState)?;
    let legacy_path = std::env::var("AYNI_LEGACY_CONTEXT_FIXTURE_PATH")
        .map_err(|_| CleanStoreError::CorruptState)?;
    let (mut store, c, mut run, profile) = setup()?;
    let scope_key = |rpc: &str, key: &str| {
        format!(
            "request-{:x}",
            Sha256::digest(format!("ayni.idempotency.v1\0trusted-local\0{rpc}\0{key}").as_bytes())
        )
    };
    let mut first = message(&c);
    first.idempotency_key = "context-first".into();
    first.content = "Decision: preserve the amber migration boundary. Open question: which research reference explains amber storage?".into();
    store.send_project_message(
        &scope_key("SendProjectMessage", &first.idempotency_key),
        &first,
        run.clone(),
    )?;
    complete(&mut store, &c, &run, &profile)?;
    let current = store.project_conversation(&c.id)?;
    let mut approval = plan_decision(&current, "approve")?;
    approval.idempotency_key = "context-approve".into();
    let approved = store.decide_project_plan(
        &scope_key("DecideProjectPlan", &approval.idempotency_key),
        &approval,
    )?;
    complete_cancelled_outcome(&mut store, &approved)?;
    let mut all_messages = store.project_conversation(&c.id)?.messages;
    append_interrupted_turns(&mut store, &c, &mut run, &mut all_messages)?;
    let mut final_message = message(&c);
    final_message.idempotency_key = "context-final".into();
    final_message.content = "Recover the amber migration decision and unresolved question, using the older research reference.".into();
    run.id = "context-final-run".into();
    let mut current = store.send_project_message("context-final", &final_message, run.clone())?;
    all_messages.push(
        current
            .messages
            .last()
            .ok_or(CleanStoreError::CorruptState)?
            .clone(),
    );
    save_frozen_interrupted_request(&mut store, &current, &run, &path)?;
    current = store.project_conversation(&c.id)?;
    let context = store.project_conversation_context(&c.project_id, &c.id)?;
    store
        .connection
        .execute("VACUUM INTO ?1", [&path])
        .map_err(CleanStoreError::Database)?;
    // Reconstruct a valid schema-21 payload before exporting the migration input.
    // No production database or running writer is modified by this exporter.
    current.messages = all_messages;
    store
        .connection
        .execute(
            "UPDATE project_conversations SET payload=?1 WHERE id=?2",
            params![current.encode_to_vec(), c.id],
        )
        .map_err(CleanStoreError::Database)?;
    store.connection.execute_batch("DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; UPDATE clean_schema SET version=21 WHERE component='control_plane';").map_err(CleanStoreError::Database)?;
    store
        .connection
        .execute("VACUUM INTO ?1", [&legacy_path])
        .map_err(CleanStoreError::Database)?;
    let manifest = serde_json::json!({"conversationId":c.id,"projectId":c.project_id,"taskId":approved.proposals[0].task_id,"attemptId":approved.proposals[0].attempt_id,"planId":approval.plan_id,"approvedVersion":"1","approvedIdempotencyKey":approval.idempotency_key,"firstMessage":{"idempotencyKey":first.idempotency_key,"conversationId":first.conversation_id,"content":first.content},"firstRecordId":format!("message:{}",current.messages[0].id),"frozenRunId":run.id,"recordCount":context.archive_record_count});
    std::fs::write(
        format!("{path}.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|_| CleanStoreError::CorruptState)?,
    )
    .map_err(CleanStoreError::CreateDirectory)?;
    Ok(())
}

fn complete_cancelled_outcome(
    store: &mut CleanStore,
    approved: &v1::ProjectConversation,
) -> Result<(), CleanStoreError> {
    store.cancel_development_task(
        "context-cancel",
        &v1::CancelDevelopmentTaskRequest {
            idempotency_key: "context-cancel".into(),
            development_task_id: approved.proposals[0].task_id.clone(),
        },
    )?;
    summary_defaults(store, approved)?;
    store.queue_project_outcome_summaries()?;
    let summary_run = store
        .queued_project_runs()?
        .into_iter()
        .next()
        .ok_or(CleanStoreError::CorruptState)?
        .1;
    store.complete_project_run(ProjectAgentResult {run_id:summary_run.id,message:"Execution cancelled. Quality and publication did not execute. Research reference: docs/storage.md at the frozen repository revision.".into(),proposals:vec![],model_calls:1,token_usage:ayni_orchestrator_domain::ProviderTokenUsage::default()})?;
    Ok(())
}
fn append_interrupted_turns(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    run: &mut v1::ProjectAgentRun,
    all_messages: &mut Vec<v1::ProjectMessage>,
) -> Result<(), CleanStoreError> {
    for index in 0..80 {
        run.id = format!("context-turn-{index}");
        let request = v1::SendProjectMessageRequest {
            idempotency_key: format!("context-send-{index}"),
            conversation_id: c.id.clone(),
            content: format!(
                "Later conversation turn {index}: discuss the ordinary project requirements."
            ),
        };
        let saved =
            store.send_project_message(&format!("context-send-{index}"), &request, run.clone())?;
        all_messages.push(
            saved
                .messages
                .last()
                .ok_or(CleanStoreError::CorruptState)?
                .clone(),
        );
        store.fail_project_run(&run.id, "project_execution_lost")?;
    }
    Ok(())
}

fn save_frozen_interrupted_request(
    store: &mut CleanStore,
    current: &v1::ProjectConversation,
    run: &v1::ProjectAgentRun,
    path: &str,
) -> Result<(), CleanStoreError> {
    let frozen = store.frozen_conversation_context(current)?;
    let mut frozen_request = store
        .project_run_request("run-research")?
        .ok_or(CleanStoreError::CorruptState)?;
    frozen_request.run_id = run.id.clone();
    frozen_request.conversation_context = frozen;
    frozen_request.history = "Recover the amber migration decision and unresolved question.".into();
    store.save_project_run_request(&run.id, &frozen_request)?;
    std::fs::write(
        format!("{path}.request.json"),
        serde_json::to_vec(&frozen_request).map_err(|_| CleanStoreError::CorruptState)?,
    )
    .map_err(CleanStoreError::CreateDirectory)?;
    store.mark_project_run_running(&run.id)?;
    store.fail_project_run(&run.id, "project_execution_lost")?;
    Ok(())
}
