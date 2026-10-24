//! Fresh-conversation context shares facts while preserving private context and exact replay.
use ayni_orchestrator_events::{
    task_execution::{SupervisorRequest as S, TaskOutcome, WorkerRequest as W},
    workflow::decode_command,
};
use ayni_orchestrator_workflow_postgres::{
    WorkflowStore, connect,
    execution::{ExecutionInputs, TaskExecutionPreparation},
    migrate_for_runtime,
    model::{CommandReceipt, Conversation, Project, Workspace},
    plans::{PlanContent, PlanScope, PlanTaskSpecification},
};
use uuid::Uuid;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn key() -> String {
    Uuid::now_v7().to_string()
}

struct Fixture {
    store: WorkflowStore,
    runtime: String,
    workspace: Workspace,
    project: Project,
    origin: Conversation,
    prepared: TaskExecutionPreparation,
    pod_uid: String,
}
impl Fixture {
    fn scope(&self) -> PlanScope<'_> {
        PlanScope {
            workspace: &self.workspace.id,
            project: &self.project.id,
            conversation: &self.origin.id,
        }
    }
}
async fn fixture() -> Result<Fixture> {
    let runtime = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime.clone());
    let workspace = store.create_workspace(&key(), "Continuity").await?.resource;
    let project = store
        .create_project(&key(), &workspace.id, "Project", "Shared objective")
        .await?
        .resource;
    let origin = store
        .create_conversation(&key(), &workspace.id, &project.id, "PRIVATE_TITLE")
        .await?
        .resource;
    store
        .send_message(
            &key(),
            &workspace.id,
            &project.id,
            &origin.id,
            "PRIVATE_TRANSCRIPT",
        )
        .await?;
    let prepared = prepare(
        &store,
        PlanScope {
            workspace: &workspace.id,
            project: &project.id,
            conversation: &origin.id,
        },
    )
    .await?;
    Ok(Fixture {
        store,
        runtime,
        workspace,
        project,
        origin,
        prepared,
        pod_uid: key(),
    })
}
async fn prepare(store: &WorkflowStore, scope: PlanScope<'_>) -> Result<TaskExecutionPreparation> {
    let version = store
        .create_plan(
            scope,
            &key(),
            "",
            &PlanContent {
                title: "PRIVATE_PLAN".into(),
                objective: "PRIVATE_REASONING".into(),
                scope: "Only origin".into(),
                tasks: (0..22)
                    .map(|i| PlanTaskSpecification {
                        key: format!("task{i}"),
                        objective: "PRIVATE_TASK_TEXT".into(),
                        scope: "Bounded".into(),
                        acceptance_criteria: vec!["Evidence".into()],
                        repository: "https://example.invalid/continuity.git".into(),
                        source_ref: "refs/heads/main".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
        )
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
    let prepared = store
        .prepare_task_execution(
            scope,
            &key(),
            &approved.tasks[0].id,
            &ExecutionInputs {
                repository: "https://example.invalid/continuity.git".into(),
                source_ref: "refs/heads/main".into(),
                source_commit: "a".repeat(40),
                lock_digest: format!("sha256:{}", "b".repeat(64)),
                runtime_image: format!("example.invalid/runtime@sha256:{}", "c".repeat(64)),
                max_seconds: 600,
                max_tokens: 10000,
            },
        )
        .await?;
    Ok(prepared)
}
async fn accept_coding(f: &Fixture) -> Result {
    let Fixture {
        store,
        prepared,
        pod_uid,
        ..
    } = f;
    let scope = f.scope();
    store
        .start_task_execution(
            scope,
            &prepared.task_id,
            &prepared.attempt_id,
            &key(),
            b"fixture-pack",
        )
        .await?;
    let claimed = store
        .task_supervisor(&S::Poll)
        .await
        .map_err(|e| format!("claim: {e}"))?
        .ok_or("claim missing")?;
    assert_eq!(
        claimed.command.attempt_id, prepared.attempt_id,
        "fixture must own the serial claim"
    );
    store
        .task_supervisor(&S::Observe {
            attempt_id: prepared.attempt_id.clone(),
            generation: 1,
            pod_uid: pod_uid.clone(),
        })
        .await
        .map_err(|e| format!("accept coding report/observe: {e}"))?;
    store
        .task_worker(
            &prepared.attempt_id,
            1,
            pod_uid,
            &W::Report {
                outcome: TaskOutcome {
                    state: "completed".into(),
                    patch: "PRIVATE_PATCH".into(),
                    ..Default::default()
                },
            },
        )
        .await
        .map_err(|e| format!("accept coding report/observe: {e}"))?;
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL with separate runtime role"]
async fn continuity_is_scoped_bounded_and_frozen_with_separate_accepted_outcomes() -> Result {
    let f = fixture().await?;
    accept_coding(&f).await?;
    let Fixture {
        store,
        runtime,
        workspace,
        project,
        prepared,
        ..
    } = &f;
    let fresh = store
        .create_conversation(&key(), &workspace.id, &project.id, "Fresh")
        .await?
        .resource;
    let replay_key = key();
    let receipt = store
        .send_message(
            &replay_key,
            &workspace.id,
            &project.id,
            &fresh.id,
            "Inspect facts",
        )
        .await?;
    let (client, driver) = connect(runtime).await?;
    let before = command_bytes(&client, &receipt.turn_id).await?;
    let (_, _, command) = decode_command(&before)?;
    let facts = command.project_status.ok_or("project facts")?;
    assert_eq!(facts.objective, "Shared objective");
    assert_eq!(facts.status, "registered");
    assert_eq!(facts.total_tasks, Some(22));
    assert!(facts.has_more_tasks);
    assert_eq!(facts.tasks.len(), 20);
    let fact = facts
        .tasks
        .iter()
        .find(|v| v.task_id == prepared.task_id)
        .ok_or("latest prepared task")?;
    assert_eq!(fact.coding_state, "reporting");
    assert_eq!(fact.accepted_coding_state, "completed");
    assert!(fact.quality_state.is_empty());
    for forbidden in [
        "PRIVATE_TITLE",
        "PRIVATE_TRANSCRIPT",
        "PRIVATE_PLAN",
        "PRIVATE_REASONING",
        "PRIVATE_TASK_TEXT",
        "PRIVATE_PATCH",
    ] {
        assert!(!String::from_utf8_lossy(&before).contains(forbidden));
    }
    assert_eq!(command.context.len(), 1);
    assert_eq!(command.context[0].content, "Inspect facts");
    cancel_after_coding(&f).await?;
    replay_and_current(&f, &client, &fresh, &replay_key, &receipt, &before).await?;
    foreign_scope(&f, &client).await?;
    driver.abort();
    Ok(())
}
async fn cancel_after_coding(f: &Fixture) -> Result {
    let Fixture {
        store,
        prepared,
        pod_uid,
        ..
    } = f;
    let scope = f.scope();
    store
        .cancel_task_execution(scope, &prepared.task_id, &prepared.attempt_id, &key())
        .await
        .map_err(|e| format!("cancel/stop accepted coding: {e}"))?;
    let cancelled = store
        .get_task_run(scope, &prepared.task_id, &prepared.attempt_id)
        .await
        .map_err(|e| format!("cancel/stop accepted coding: {e}"))?;
    store
        .task_supervisor(&S::Stopped {
            attempt_id: prepared.attempt_id.clone(),
            generation: cancelled.command.generation,
            pod_uid: pod_uid.clone(),
        })
        .await
        .map_err(|e| format!("cancel/stop accepted coding: {e}"))?;
    Ok(())
}
async fn replay_and_current(
    f: &Fixture,
    client: &tokio_postgres::Client,
    fresh: &Conversation,
    replay_key: &str,
    receipt: &CommandReceipt,
    before: &[u8],
) -> Result {
    let Fixture {
        store,
        runtime,
        workspace,
        project,
        prepared,
        ..
    } = f;
    let reconnected = WorkflowStore::new(runtime.clone());
    assert_eq!(
        reconnected
            .send_message(
                replay_key,
                &workspace.id,
                &project.id,
                &fresh.id,
                "Inspect facts"
            )
            .await?,
        *receipt
    );
    assert_eq!(command_bytes(client, &receipt.turn_id).await?, before);
    let next = store
        .create_conversation(&key(), &workspace.id, &project.id, "Later")
        .await?
        .resource;
    let next = store
        .send_message(
            &key(),
            &workspace.id,
            &project.id,
            &next.id,
            "Inspect current facts",
        )
        .await?;
    let (_, _, command) = decode_command(&command_bytes(client, &next.turn_id).await?)?;
    let facts = command.project_status.ok_or("facts")?;
    let fact = facts
        .tasks
        .iter()
        .find(|v| v.task_id == prepared.task_id)
        .ok_or("task")?;
    assert_eq!(fact.coding_state, "cancelled");
    assert_eq!(fact.accepted_coding_state, "completed");
    assert_eq!(fact.quality_state, "interrupted");
    Ok(())
}
async fn foreign_scope(f: &Fixture, client: &tokio_postgres::Client) -> Result {
    let Fixture {
        store, workspace, ..
    } = f;
    let foreign = store
        .create_project(&key(), &workspace.id, "Foreign", "Other project")
        .await?
        .resource;
    let foreign_conversation = store
        .create_conversation(&key(), &workspace.id, &foreign.id, "Foreign")
        .await?
        .resource;
    let foreign_turn = store
        .send_message(
            &key(),
            &workspace.id,
            &foreign.id,
            &foreign_conversation.id,
            "Only own facts",
        )
        .await?;
    let (_, _, foreign_command) =
        decode_command(&command_bytes(client, &foreign_turn.turn_id).await?)?;
    let facts = foreign_command.project_status.ok_or("foreign facts")?;
    assert_eq!(facts.total_tasks, Some(0));
    assert!(facts.tasks.is_empty());
    assert!(!facts.has_more_tasks);
    Ok(())
}
async fn command_bytes(client: &tokio_postgres::Client, turn: &str) -> Result<Vec<u8>> {
    Ok(client
        .query_one(
            "SELECT exact_command_bytes FROM workflow_dispatch_intents WHERE turn_id=$1",
            &[&turn],
        )
        .await?
        .get(0))
}
