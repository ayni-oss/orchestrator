//! Preparation is durable authorization of inputs, never dispatch authority.
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect,
    execution::ExecutionInputs,
    migrate_for_runtime,
    plans::{PlanContent, PlanScope, PlanTaskSpecification},
};
use std::error::Error;
use uuid::Uuid;
type Result = std::result::Result<(), Box<dyn Error + Send + Sync>>;
fn key() -> String {
    Uuid::now_v7().to_string()
}
fn inputs() -> ExecutionInputs {
    ExecutionInputs {
        repository: "https://example.invalid/project.git".into(),
        source_ref: "refs/heads/main".into(),
        source_commit: "a".repeat(40),
        lock_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_image: format!("example.invalid/runtime@sha256:{}", "c".repeat(64)),
        max_seconds: 600,
        max_tokens: 10000,
        upstream_selections: vec![],
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and distinct runtime role"]
#[allow(
    clippy::too_many_lines,
    reason = "one scenario proves atomic preparation through retries, races, rollback and restart"
)]
async fn preparation_freezes_one_attempt_preserves_origin_and_never_dispatches() -> Result {
    let admin_url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &admin_url,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime_url.clone());
    let workspace = store
        .create_workspace(&key(), "Execution workspace")
        .await?
        .resource;
    let project = store
        .create_project(&key(), &workspace.id, "Execution", "Prepare frozen tasks")
        .await?
        .resource;
    let conversation = store
        .create_conversation(&key(), &workspace.id, &project.id, "Owner")
        .await?
        .resource;
    let other = store
        .create_conversation(&key(), &workspace.id, &project.id, "Other")
        .await?
        .resource;
    let scope = PlanScope {
        workspace: &workspace.id,
        project: &project.id,
        conversation: &conversation.id,
    };
    let wrong = PlanScope {
        conversation: &other.id,
        ..scope
    };
    let content = PlanContent {
        title: "Prepare execution".into(),
        objective: "Freeze approved inputs".into(),
        scope: "One task".into(),
        tasks: ["first", "second", "question"]
            .iter()
            .map(|key| PlanTaskSpecification {
                key: (*key).into(),
                objective: "Frozen task".into(),
                scope: "Approved scope".into(),
                acceptance_criteria: vec!["No dispatch".into()],
                repository: inputs().repository,
                source_ref: inputs().source_ref,
                unresolved_inputs: if *key == "question" {
                    vec!["Select deployment target".into()]
                } else {
                    vec![]
                },
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let version = store
        .create_plan(scope, &key(), "", &content)
        .await?
        .version;
    let approved = store
        .approve_plan(
            scope,
            &key(),
            &version.plan_id,
            &version.id,
            &version.content_digest,
        )
        .await?;
    let task = &approved.tasks[0].id;
    assert!(store.get_task_execution(scope, task).await?.is_none());
    assert!(matches!(
        store
            .prepare_task_execution(wrong, &key(), task, &inputs())
            .await,
        Err(StoreError::NotFound)
    ));
    let missing_key = key();
    let blocked = store
        .prepare_task_execution(scope, &missing_key, task, &ExecutionInputs::default())
        .await?;
    assert!(blocked.attempt_id.is_empty());
    assert_eq!(blocked.blockers.len(), 4);
    assert_eq!(
        store
            .prepare_task_execution(scope, &missing_key, task, &ExecutionInputs::default())
            .await?,
        blocked
    );
    let mut changed = inputs();
    changed.repository = "https://example.invalid/other.git".into();
    assert!(matches!(
        store
            .prepare_task_execution(scope, &key(), task, &changed)
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    let unresolved = store
        .prepare_task_execution(scope, &key(), &approved.tasks[2].id, &inputs())
        .await?;
    assert!(unresolved.attempt_id.is_empty());
    assert_eq!(unresolved.blockers, ["Select deployment target"]);
    let first_key = key();
    let second_key = key();
    let frozen = inputs();
    let (a, b) = tokio::join!(
        store.prepare_task_execution(scope, &first_key, task, &frozen),
        store.prepare_task_execution(scope, &second_key, task, &frozen)
    );
    let (prepared, winner) = match (a, b) {
        (Ok(a), Err(StoreError::TaskAlreadyPrepared)) => (a, first_key),
        (Err(StoreError::TaskAlreadyPrepared), Ok(b)) => (b, second_key),
        other => return Err(format!("unexpected race: {other:?}").into()),
    };
    assert!(!prepared.attempt_id.is_empty());
    assert!(prepared.blockers.is_empty());
    assert_eq!(prepared.inputs, frozen);
    let mut config = store.get_agent_configuration(&workspace.id, "").await?;
    assert_eq!(prepared.configuration_revision_id, config.revision_id);
    let prior = config.revision_id.clone();
    config.revision_id.clear();
    config.instructions = "Later configuration".into();
    store
        .update_agent_configuration(&key(), &prior, &config)
        .await?;
    let reopened = WorkflowStore::new(runtime_url.clone());
    assert_eq!(
        reopened
            .prepare_task_execution(scope, &winner, task, &frozen)
            .await?,
        prepared
    );
    assert_eq!(
        reopened.get_task_execution(scope, task).await?,
        Some(prepared.clone())
    );
    assert_eq!(
        reopened
            .prepare_task_execution(scope, &missing_key, task, &ExecutionInputs::default())
            .await?,
        blocked
    );
    assert_eq!(
        reopened.get_task_execution(scope, task).await?,
        Some(prepared.clone()),
        "old replay must not replace latest readiness"
    );
    let mut changed = frozen.clone();
    changed.max_tokens += 1;
    assert!(matches!(
        store
            .prepare_task_execution(scope, &winner, task, &changed)
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store.get_task_execution(wrong, task).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        store.get_plan_approval(scope, &version.plan_id).await?,
        Some(approved.clone())
    );
    let (admin, admin_driver) = connect(&admin_url).await?;
    let row = admin.query_one("SELECT workspace_id,project_id,conversation_id,plan_id,version_id,content_digest,approval_id,state FROM workflow_task_attempts WHERE id=$1", &[&prepared.attempt_id]).await?;
    let origin: Vec<String> = (0..8).map(|i| row.get(i)).collect();
    assert_eq!(
        origin,
        [
            workspace.id.clone(),
            project.id.clone(),
            conversation.id.clone(),
            version.plan_id.clone(),
            version.id.clone(),
            version.content_digest.clone(),
            approved.approval.id.clone(),
            "prepared".into()
        ]
    );
    assert_eq!(
        admin
            .query_one(
                "SELECT count(*) FROM workflow_task_attempts WHERE task_id=$1",
                &[&task]
            )
            .await?
            .get::<_, i64>(0),
        1
    );
    for table in [
        "workflow_turns",
        "workflow_dispatch_intents",
        "workflow_outbox",
    ] {
        assert_eq!(
            admin
                .query_one(
                    &format!("SELECT count(*) FROM {table} WHERE conversation_id=$1"),
                    &[&conversation.id]
                )
                .await?
                .get::<_, i64>(0),
            0,
            "preparation dispatched through {table}"
        );
    }
    let (runtime, runtime_driver) = connect(&runtime_url).await?;
    for sql in [
        "UPDATE workflow_task_attempts SET state='prepared' WHERE id=$1",
        "DELETE FROM workflow_task_attempts WHERE id=$1",
    ] {
        assert!(runtime.execute(sql, &[&prepared.attempt_id]).await.is_err());
    }
    assert!(
        admin
            .execute(
                "UPDATE workflow_task_attempts SET conversation_id=$2 WHERE id=$1",
                &[&prepared.attempt_id, &other.id]
            )
            .await
            .is_err()
    );
    // Force failure after preparation/update insertion: the whole command rolls back.
    admin.batch_execute("CREATE FUNCTION fixture_reject_attempt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture attempt failure'; END $$; CREATE TRIGGER fixture_reject_attempt BEFORE INSERT ON workflow_task_attempts FOR EACH ROW EXECUTE FUNCTION fixture_reject_attempt()").await?;
    let failure_key = key();
    let failed = store
        .prepare_task_execution(scope, &failure_key, &approved.tasks[1].id, &frozen)
        .await;
    admin.batch_execute("DROP TRIGGER fixture_reject_attempt ON workflow_task_attempts; DROP FUNCTION fixture_reject_attempt()").await?;
    assert!(matches!(failed, Err(StoreError::Database(_))));
    assert!(
        store
            .get_task_execution(scope, &approved.tasks[1].id)
            .await?
            .is_none()
    );
    assert!(
        !store
            .prepare_task_execution(scope, &failure_key, &approved.tasks[1].id, &frozen)
            .await?
            .attempt_id
            .is_empty()
    );
    runtime_driver.abort();
    admin_driver.abort();
    Ok(())
}
