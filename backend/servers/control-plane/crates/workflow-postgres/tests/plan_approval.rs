use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect, migrate_for_runtime,
    plans::{PlanContent, PlanScope, PlanTaskSpecification},
};
use std::error::Error;
use uuid::Uuid;
type TestResult = Result<(), Box<dyn Error + Send + Sync>>;
fn key() -> String {
    Uuid::now_v7().to_string()
}
fn content(title: &str) -> PlanContent {
    PlanContent {
        title: title.into(),
        objective: "Authorize reviewed work".into(),
        scope: "Bounded scope".into(),
        tasks: vec![
            PlanTaskSpecification {
                key: "first".into(),
                objective: "First task".into(),
                scope: "First scope".into(),
                acceptance_criteria: vec!["Observable outcome".into()],
                unresolved_inputs: vec!["Select source commit".into()],
                ..Default::default()
            },
            PlanTaskSpecification {
                key: "second".into(),
                objective: "Second task".into(),
                scope: "Second scope".into(),
                acceptance_criteria: vec!["Second outcome".into()],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and distinct runtime role"]
#[allow(
    clippy::too_many_lines,
    reason = "one durable scenario follows approval through races and restart"
)]
async fn approval_retains_one_complete_set_and_origin_without_dispatch() -> TestResult {
    let admin_url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &admin_url,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime_url.clone());
    let workspace = store
        .create_workspace(&key(), "Approval workspace")
        .await?
        .resource;
    let project = store
        .create_project(&key(), &workspace.id, "Approval project", "Retain origin")
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
    let draft = content("Reviewed");
    let created = store.create_plan(scope, &key(), "", &draft).await?;
    let plan = &created.version.plan_id;
    assert!(store.get_plan_approval(scope, plan).await?.is_none());
    assert!(matches!(
        store
            .approve_plan(
                wrong,
                &key(),
                plan,
                &created.version.id,
                &created.version.content_digest
            )
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store
            .approve_plan(
                scope,
                &key(),
                plan,
                &created.version.id,
                &format!("sha256:{}", "0".repeat(64))
            )
            .await,
        Err(StoreError::PlanDigestMismatch)
    ));
    let approval_key = key();
    let approval = store
        .approve_plan(
            scope,
            &approval_key,
            plan,
            &created.version.id,
            &created.version.content_digest,
        )
        .await?;
    assert_eq!(approval.tasks.len(), 2);
    assert_eq!(
        approval.approval.task_ids,
        approval
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(approval.approval.operator, "local-operator");
    assert_eq!(approval.approval.request_digest.len(), 71);
    for (task, specification) in approval.tasks.iter().zip(&draft.tasks) {
        assert_eq!(&task.specification, specification);
        assert_eq!(task.workspace_id, workspace.id);
        assert_eq!(task.project_id, project.id);
        assert_eq!(task.conversation_id, conversation.id);
        assert_eq!(&task.plan_id, plan);
        assert_eq!(task.version_id, created.version.id);
        assert_eq!(task.content_digest, created.version.content_digest);
        assert_eq!(task.approval_id, approval.approval.id);
        assert_eq!(task.proposed_task_key, specification.key);
        assert_eq!(task.state, "awaiting_execution");
        assert_eq!(
            task.execution_blockers.len(),
            3 + specification.unresolved_inputs.len()
        );
        assert_eq!(store.get_task(scope, &task.id).await?, *task);
        assert!(matches!(
            store.get_task(wrong, &task.id).await,
            Err(StoreError::NotFound)
        ));
    }
    let restarted = WorkflowStore::new(runtime_url.clone());
    assert_eq!(
        restarted
            .approve_plan(
                scope,
                &approval_key,
                plan,
                &created.version.id,
                &created.version.content_digest
            )
            .await?,
        approval
    );
    assert_eq!(
        restarted.get_plan_approval(scope, plan).await?,
        Some(approval.clone())
    );
    assert!(matches!(
        store
            .approve_plan(
                scope,
                &approval_key,
                plan,
                &created.version.id,
                &format!("sha256:{}", "0".repeat(64))
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .approve_plan(
                scope,
                &key(),
                plan,
                &created.version.id,
                &created.version.content_digest
            )
            .await,
        Err(StoreError::PlanAlreadyApproved)
    ));
    assert!(matches!(
        store
            .revise_plan(
                scope,
                &key(),
                plan,
                &created.version.id,
                &content("After approval")
            )
            .await,
        Err(StoreError::PlanAlreadyApproved)
    ));
    let snapshot = store
        .get_snapshot(scope.workspace, scope.project, scope.conversation)
        .await?;
    assert_eq!(snapshot.plans[0].approval_id, approval.approval.id);
    assert_eq!(
        snapshot.conversation_revision,
        approval.approval.conversation_revision
    );
    assert_eq!(snapshot.resume_cursor, approval.approval.resume_cursor);
    let page = store.list_tasks(scope, "", 1).await?;
    assert_eq!(page.tasks.len(), 1);
    assert!(!page.next_after_id.is_empty());
    let next = store.list_tasks(scope, &page.next_after_id, 1).await?;
    assert_eq!(next.tasks.len(), 1);
    assert!(next.next_after_id.is_empty());
    assert_ne!(page.tasks[0].id, next.tasks[0].id);
    assert!(store.list_tasks(wrong, "", 0).await?.tasks.is_empty());
    assert!(matches!(
        store.list_tasks(wrong, &page.next_after_id, 0).await,
        Err(StoreError::InvalidInput("after_id"))
    ));
    assert!(matches!(
        store.list_tasks(scope, "", 33).await,
        Err(StoreError::InvalidInput("task_list_limit"))
    ));
    prove_scope_eligibility(&store, scope).await?;
    prove_approval_races(&store, scope).await?;
    prove_revision_race(&store, scope).await?;
    let (admin, driver) = connect(&admin_url).await?;
    prove_rollback(&admin, &store, scope).await?;
    prove_constraints(
        &admin,
        &runtime_url,
        scope,
        wrong,
        plan,
        &approval.tasks[0].id,
    )
    .await?;
    for table in [
        "workflow_turns",
        "workflow_dispatch_intents",
        "workflow_outbox",
    ] {
        let count: i64 = admin
            .query_one(
                &format!("SELECT count(*) FROM {table} WHERE conversation_id=$1"),
                &[&conversation.id],
            )
            .await?
            .get(0);
        assert_eq!(count, 0, "approval must not dispatch through {table}");
    }
    let updates = admin.query("SELECT affected_resource_ids FROM workflow_updates WHERE conversation_id=$1 AND kind='plan_approved' AND $2=ANY(affected_resource_ids)",&[&conversation.id,&approval.approval.id]).await?;
    assert_eq!(updates.len(), 1);
    let mut affected = vec![plan.clone(), created.version.id, approval.approval.id];
    affected.extend(approval.approval.task_ids);
    assert_eq!(updates[0].get::<_, Vec<String>>(0), affected);
    driver.abort();
    println!(
        "plan-approval: immutable-origin=pass exact-restart-retry=pass scope=pass races=pass rollback=pass no-dispatch=pass"
    );
    Ok(())
}

async fn prove_scope_eligibility(store: &WorkflowStore, scope: PlanScope<'_>) -> TestResult {
    let mut unresolved = content("Unresolved scope");
    unresolved
        .unresolved_questions
        .push("Which acceptance outcome?".into());
    let mut empty = content("No tasks");
    empty.tasks.clear();
    let mut criteria = content("No acceptance");
    criteria.tasks[0].acceptance_criteria.clear();
    for draft in [unresolved, empty, criteria] {
        let created = store.create_plan(scope, &key(), "", &draft).await?;
        assert!(matches!(
            store
                .approve_plan(
                    scope,
                    &key(),
                    &created.version.plan_id,
                    &created.version.id,
                    &created.version.content_digest
                )
                .await,
            Err(StoreError::PlanUnresolvedScope)
        ));
        assert!(
            store
                .get_plan_approval(scope, &created.version.plan_id)
                .await?
                .is_none()
        );
    }
    Ok(())
}

async fn prove_approval_races(store: &WorkflowStore, scope: PlanScope<'_>) -> TestResult {
    for same_key in [true, false] {
        let created = store
            .create_plan(scope, &key(), "", &content("Approval race"))
            .await?;
        let one = key();
        let two = if same_key { one.clone() } else { key() };
        let (left, right) = tokio::join!(
            store.approve_plan(
                scope,
                &one,
                &created.version.plan_id,
                &created.version.id,
                &created.version.content_digest
            ),
            store.approve_plan(
                scope,
                &two,
                &created.version.plan_id,
                &created.version.id,
                &created.version.content_digest
            )
        );
        if same_key {
            assert_eq!(left?, right?);
        } else {
            assert!(matches!(
                (&left, &right),
                (Ok(_), Err(StoreError::PlanAlreadyApproved))
                    | (Err(StoreError::PlanAlreadyApproved), Ok(_))
            ));
        }
        assert_eq!(
            store
                .get_plan_approval(scope, &created.version.plan_id)
                .await?
                .ok_or("approval missing")?
                .tasks
                .len(),
            2
        );
    }
    Ok(())
}

async fn prove_revision_race(store: &WorkflowStore, scope: PlanScope<'_>) -> TestResult {
    let created = store
        .create_plan(scope, &key(), "", &content("Revision race"))
        .await?;
    let approval_key = key();
    let revision_key = key();
    let revised = content("New head");
    let (approved, revised) = tokio::join!(
        store.approve_plan(
            scope,
            &approval_key,
            &created.version.plan_id,
            &created.version.id,
            &created.version.content_digest
        ),
        store.revise_plan(
            scope,
            &revision_key,
            &created.version.plan_id,
            &created.version.id,
            &revised
        )
    );
    assert!(matches!(
        (&approved, &revised),
        (Ok(_), Err(StoreError::PlanAlreadyApproved))
            | (Err(StoreError::PlanVersionConflict), Ok(_))
    ));
    Ok(())
}

async fn prove_rollback(
    admin: &tokio_postgres::Client,
    store: &WorkflowStore,
    scope: PlanScope<'_>,
) -> TestResult {
    let created = store
        .create_plan(scope, &key(), "", &content("Rollback"))
        .await?;
    let before = store
        .get_snapshot(scope.workspace, scope.project, scope.conversation)
        .await?;
    // The second task fails after the receipt and first task were inserted.
    admin.batch_execute(&format!("CREATE FUNCTION approval_task_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.plan_id='{}' AND NEW.proposed_task_key='second' THEN RAISE EXCEPTION 'fixture interruption'; END IF; RETURN NEW; END $$; CREATE TRIGGER approval_task_failure BEFORE INSERT ON workflow_tasks FOR EACH ROW EXECUTE FUNCTION approval_task_failure()",created.version.plan_id)).await?;
    let command = key();
    let result = store
        .approve_plan(
            scope,
            &command,
            &created.version.plan_id,
            &created.version.id,
            &created.version.content_digest,
        )
        .await;
    admin.batch_execute("DROP TRIGGER approval_task_failure ON workflow_tasks; DROP FUNCTION approval_task_failure()").await?;
    assert!(matches!(result, Err(StoreError::Database(_))));
    assert!(
        store
            .get_plan_approval(scope, &created.version.plan_id)
            .await?
            .is_none()
    );
    let after = store
        .get_snapshot(scope.workspace, scope.project, scope.conversation)
        .await?;
    assert_eq!(before.conversation_revision, after.conversation_revision);
    assert_eq!(before.resume_cursor, after.resume_cursor);
    let count: i64 = admin
        .query_one(
            "SELECT count(*) FROM workflow_tasks WHERE plan_id=$1",
            &[&created.version.plan_id],
        )
        .await?
        .get(0);
    assert_eq!(count, 0);
    assert_eq!(
        store
            .approve_plan(
                scope,
                &command,
                &created.version.plan_id,
                &created.version.id,
                &created.version.content_digest
            )
            .await?
            .tasks
            .len(),
        2
    );
    Ok(())
}

async fn prove_constraints(
    admin: &tokio_postgres::Client,
    runtime_url: &str,
    scope: PlanScope<'_>,
    other: PlanScope<'_>,
    plan: &str,
    task: &str,
) -> TestResult {
    let (runtime, driver) = connect(runtime_url).await?;
    for sql in [
        "UPDATE workflow_tasks SET state='awaiting_execution' WHERE id=$1",
        "DELETE FROM workflow_tasks WHERE id=$1",
    ] {
        assert!(runtime.execute(sql, &[&task]).await.is_err());
    }
    assert!(
        runtime
            .execute(
                "UPDATE workflow_plan_approvals SET operator='local-operator' WHERE plan_id=$1",
                &[&plan]
            )
            .await
            .is_err()
    );
    assert!(
        admin
            .execute(
                "UPDATE workflow_tasks SET conversation_id=$2 WHERE id=$1",
                &[&task, &other.conversation]
            )
            .await
            .is_err()
    );
    assert!(
        admin
            .execute(
                "UPDATE workflow_tasks SET content_digest=$2 WHERE id=$1",
                &[&task, &format!("sha256:{}", "0".repeat(64))]
            )
            .await
            .is_err()
    );
    assert!(
        admin
            .execute(
                "DELETE FROM workflow_plan_approvals WHERE plan_id=$1",
                &[&plan]
            )
            .await
            .is_err()
    );
    assert!(
        admin
            .execute(
                "DELETE FROM workflow_conversations WHERE id=$1",
                &[&scope.conversation]
            )
            .await
            .is_err()
    );
    driver.abort();
    Ok(())
}
