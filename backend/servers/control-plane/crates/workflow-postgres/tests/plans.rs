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
        objective: "Discuss durable review".into(),
        scope: "This conversation".into(),
        unresolved_questions: vec!["Which source commit?".into()],
        tasks: vec![PlanTaskSpecification {
            key: "review".into(),
            objective: "Review scope".into(),
            scope: "Plan review".into(),
            acceptance_criteria: vec!["Retained after restart".into()],
            repository: "declared/example".into(),
            source_ref: "unverified-source".into(),
            unresolved_inputs: vec!["Verify repository and budget before execution".into()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and distinct runtime role"]
#[allow(
    clippy::too_many_lines,
    reason = "the cumulative recovery scenario keeps command ordering explicit"
)]
async fn plan_review_preserves_scope_replay_races_history_and_atomicity() -> TestResult {
    let admin_url = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime_url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &admin_url,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime_url.clone());
    let workspace = store
        .create_workspace(&key(), "Plan workspace")
        .await?
        .resource;
    let project = store
        .create_project(&key(), &workspace.id, "Plan project", "Retain origin")
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
    let create_key = key();
    let initial = content("Original <script>never executed</script>");
    let created = store.create_plan(scope, &create_key, "", &initial).await?;
    assert_eq!(created.version.content, initial);
    assert_eq!(created.version.author, "local-operator");
    assert_eq!(created.version.conversation_id, conversation.id);
    assert_eq!(created.version.project_id, project.id);
    let revision_key = key();
    let edited = content("Revised");
    let revised = store
        .revise_plan(
            scope,
            &revision_key,
            &created.version.plan_id,
            &created.version.id,
            &edited,
        )
        .await?;
    assert_eq!(revised.version.ordinal, 2);
    assert_eq!(revised.version.predecessor_id, created.version.id);
    let restarted = WorkflowStore::new(runtime_url.clone());
    assert_eq!(
        restarted
            .create_plan(scope, &create_key, "", &initial)
            .await?,
        created
    );
    assert_eq!(
        restarted
            .revise_plan(
                scope,
                &revision_key,
                &created.version.plan_id,
                &created.version.id,
                &edited
            )
            .await?,
        revised
    );
    assert!(matches!(
        store.create_plan(scope, &create_key, "", &edited).await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store
            .get_plan_version(wrong, &created.version.plan_id, "")
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(store.list_plans(wrong).await?.is_empty());
    assert!(matches!(
        store
            .create_plan(wrong, &key(), &created.version.plan_id, &initial)
            .await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        store
            .get_plan_version(scope, &created.version.plan_id, &created.version.id)
            .await?,
        created.version
    );
    let k1 = key();
    let k2 = key();
    let one = content("Race one");
    let two = content("Race two");
    let (left, right) = tokio::join!(
        store.revise_plan(
            scope,
            &k1,
            &created.version.plan_id,
            &revised.version.id,
            &one
        ),
        store.revise_plan(
            scope,
            &k2,
            &created.version.plan_id,
            &revised.version.id,
            &two
        )
    );
    assert!(matches!(
        (&left, &right),
        (Ok(_), Err(StoreError::PlanVersionConflict))
            | (Err(StoreError::PlanVersionConflict), Ok(_))
    ));
    let page = store
        .list_plan_versions(scope, &created.version.plan_id, 0, 2)
        .await?;
    assert_eq!(
        page.versions.iter().map(|v| v.ordinal).collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert_eq!(
        store
            .list_plan_versions(scope, &created.version.plan_id, page.next_before_ordinal, 2)
            .await?
            .versions[0]
            .id,
        created.version.id
    );
    let snapshot = store
        .get_snapshot(&workspace.id, &project.id, &conversation.id)
        .await?;
    assert_eq!(snapshot.plans.len(), 1);
    assert_eq!(snapshot.plans[0].current_ordinal, 3);
    let (admin, driver) = connect(&admin_url).await?;
    prove_transaction_rollback(&admin, &store, scope).await?;
    prove_ownership_and_immutability(&admin, &runtime_url, scope, wrong, &created.version.plan_id)
        .await?;
    prove_bounds(&store, scope, &created.version.plan_id).await?;
    let events=admin.query("SELECT kind,affected_resource_ids FROM workflow_updates WHERE conversation_id=$1 AND kind='plan_version_created' ORDER BY cursor", &[&conversation.id]).await?;
    assert_eq!(events.len(), 95);
    assert_eq!(
        events[0].get::<_, Vec<String>>(1),
        vec![created.version.plan_id, created.version.id]
    );
    assert_eq!(
        admin
            .query_one(
                "SELECT count(*) FROM workflow_turns WHERE conversation_id=$1",
                &[&conversation.id]
            )
            .await?
            .get::<_, i64>(0),
        0
    );
    driver.abort();
    println!(
        "plan-review: immutable=pass origin=pass restart-replay=pass race=pass rollback=pass bounds=pass no-execution=pass"
    );
    Ok(())
}

async fn prove_transaction_rollback(
    admin: &tokio_postgres::Client,
    store: &WorkflowStore,
    scope: PlanScope<'_>,
) -> TestResult {
    let before = store.list_plans(scope).await?;
    let revision = store
        .get_snapshot(scope.workspace, scope.project, scope.conversation)
        .await?
        .conversation_revision;
    admin.batch_execute(&format!("CREATE FUNCTION plan_receipt_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.conversation_id='{}' THEN RAISE EXCEPTION 'fixture interruption'; END IF; RETURN NEW; END $$; CREATE TRIGGER plan_receipt_failure BEFORE INSERT ON workflow_plan_receipts FOR EACH ROW EXECUTE FUNCTION plan_receipt_failure()",scope.conversation)).await?;
    let command = key();
    let proposal = content("Rollback");
    let result = store.create_plan(scope, &command, "", &proposal).await;
    admin.batch_execute("DROP TRIGGER plan_receipt_failure ON workflow_plan_receipts; DROP FUNCTION plan_receipt_failure()").await?;
    assert!(matches!(result, Err(StoreError::Database(_))));
    assert_eq!(store.list_plans(scope).await?, before);
    assert_eq!(
        store
            .get_snapshot(scope.workspace, scope.project, scope.conversation)
            .await?
            .conversation_revision,
        revision
    );
    let accepted = store.create_plan(scope, &command, "", &proposal).await?;
    assert_eq!(accepted.version.ordinal, 1);
    Ok(())
}

async fn prove_ownership_and_immutability(
    admin: &tokio_postgres::Client,
    runtime_url: &str,
    scope: PlanScope<'_>,
    other: PlanScope<'_>,
    plan: &str,
) -> TestResult {
    let (runtime, driver) = connect(runtime_url).await?;
    assert!(
        runtime
            .execute(
                "UPDATE workflow_plan_versions SET content='bad' WHERE plan_id=$1",
                &[&plan]
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .execute("DELETE FROM workflow_plans WHERE id=$1", &[&plan])
            .await
            .is_err()
    );
    assert!(
        runtime
            .execute(
                "UPDATE workflow_plans SET conversation_id=$2 WHERE id=$1",
                &[&plan, &other.conversation]
            )
            .await
            .is_err()
    );
    assert!(
        admin
            .execute(
                "UPDATE workflow_plans SET conversation_id=$2 WHERE id=$1",
                &[&plan, &other.conversation]
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

async fn prove_bounds(store: &WorkflowStore, scope: PlanScope<'_>, plan: &str) -> TestResult {
    let mut head = store.get_plan_version(scope, plan, "").await?;
    let mut last = None;
    for ordinal in 4..=64 {
        let k = key();
        let c = content(&format!("Version {ordinal}"));
        let expected = head.id.clone();
        let result = store.revise_plan(scope, &k, plan, &expected, &c).await?;
        head = result.version.clone();
        last = Some((k, expected, c, result));
    }
    assert!(matches!(
        store
            .revise_plan(scope, &key(), plan, &head.id, &content("Too many"))
            .await,
        Err(StoreError::Capacity("plan_versions"))
    ));
    let (k, expected, c, result) = last.ok_or("last revision missing")?;
    assert_eq!(
        store.revise_plan(scope, &k, plan, &expected, &c).await?,
        result
    );
    for _ in 2..32 {
        store
            .create_plan(scope, &key(), "", &content("Another plan"))
            .await?;
    }
    assert_eq!(store.list_plans(scope).await?.len(), 32);
    assert!(matches!(
        store
            .create_plan(scope, &key(), "", &content("Too many plans"))
            .await,
        Err(StoreError::Capacity("conversation_plans"))
    ));
    Ok(())
}
