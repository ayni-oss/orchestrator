use super::*;
use ayni_orchestrator_events::progress::ExecutionProgress;

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn provisional_progress_is_ordered_fenced_and_removed_on_terminal_or_cancel() -> TestResult {
    let f = Fixture::new().await?;
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    let mut request = call.request("progress");
    request.progress = Some(ExecutionProgress {
        sequence: 1,
        call_id: call.evidence.call_id.clone(),
        stage: "model".into(),
        provisional_text: "Unfinished".into(),
        ..Default::default()
    });
    assert!(f.store.session_authority(&request).await?.allowed);
    let first = f
        .store
        .list_execution_progress(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].progress.provisional_text, "Unfinished");
    assert_eq!(first[0].generation, 1);
    let before = f
        .store
        .get_snapshot(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert!(
        before
            .recent_messages
            .iter()
            .all(|message| message.content != "Unfinished")
    );
    f.store.session_authority(&request).await?;
    let exact = f
        .store
        .get_snapshot(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert_eq!(before.conversation_revision, exact.conversation_revision);
    request
        .progress
        .as_mut()
        .ok_or("progress")?
        .provisional_text = "Changed same sequence".into();
    assert!(matches!(
        f.store.session_authority(&request).await,
        Err(StoreError::InvalidInput("changed_progress"))
    ));
    request.progress.as_mut().ok_or("progress")?.sequence = 2;
    f.store.session_authority(&request).await?;
    assert!(
        f.store
            .session_authority(&call.request("admit"))
            .await?
            .allowed
    );
    assert!(
        f.store
            .accept_terminal_report(&call.terminal("completed", Some(7))?)
            .await?
            .accepted
    );
    assert!(
        f.store
            .list_execution_progress(&f.workspace, &f.project, &f.conversation)
            .await?
            .is_empty()
    );
    let count: i64 = f
        .client
        .query_one(
            "SELECT count(*) FROM workflow_execution_progress WHERE turn_id=$1",
            &[&turn],
        )
        .await?
        .get(0);
    assert_eq!(count, 0);
    // The storage fixture models completed broker delivery before replacement.
    f.client
        .execute(
            "UPDATE workflow_outbox SET state='transferred' WHERE conversation_id=$1",
            &[&f.conversation],
        )
        .await?;
    f.store
        .replace_idle_session(&f.workspace, &f.project, &f.conversation, 1, &key())
        .await?;
    assert!(matches!(
        f.store.session_authority(&request).await,
        Err(StoreError::InvalidInput("stale_command"))
    ));
    prove_second_generation(&f).await
}

async fn prove_second_generation(f: &Fixture) -> TestResult {
    let turn = f.send().await?;
    let call = f.claim(&turn).await?;
    let mut request = call.request("progress");
    request.progress = Some(ExecutionProgress {
        sequence: 1,
        call_id: call.evidence.call_id.clone(),
        stage: "model".into(),
        provisional_text: "Second generation".into(),
        ..Default::default()
    });
    assert!(f.store.session_authority(&request).await?.allowed);
    let page = f
        .store
        .list_execution_progress(&f.workspace, &f.project, &f.conversation)
        .await?;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].generation, 2);
    assert!(matches!(
        f.store
            .list_execution_progress(&f.workspace, &key(), &f.conversation)
            .await,
        Err(StoreError::NotFound)
    ));
    request.progress.as_mut().ok_or("progress")?.sequence = 2;
    request
        .progress
        .as_mut()
        .ok_or("progress")?
        .provisional_text = "x".repeat(16385);
    assert!(matches!(
        f.store.session_authority(&request).await,
        Err(StoreError::InvalidInput("execution_progress"))
    ));
    f.cancel(&turn, &key()).await?;
    assert!(
        f.store
            .list_execution_progress(&f.workspace, &f.project, &f.conversation)
            .await?
            .is_empty()
    );
    assert!(!f.store.session_authority(&request).await?.allowed);
    let count: i64 = f
        .client
        .query_one(
            "SELECT count(*) FROM workflow_execution_progress WHERE turn_id=$1",
            &[&turn],
        )
        .await?
        .get(0);
    assert_eq!(count, 0, "late progress recreated cancelled text");
    Ok(())
}
