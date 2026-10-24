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
                upstream_selections: vec![],
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

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with separate runtime role"]
#[allow(
    clippy::too_many_lines,
    reason = "one retained consent, coalescing and revocation lifecycle scenario"
)]
async fn assessments_require_bounded_consent_coalesce_and_replay_without_extra_turns() -> Result {
    let f = fixture().await?;
    let configuration = f.store.get_agent_configuration(&f.workspace.id, "").await?;
    let consent_key = key();
    let consent = f
        .store
        .authorize_outcome_assessments(f.scope(), &consent_key, &configuration.revision_id, 2)
        .await?;
    assert_eq!(
        f.store
            .authorize_outcome_assessments(f.scope(), &consent_key, &configuration.revision_id, 2)
            .await?,
        consent
    );
    assert!(
        f.store
            .authorize_outcome_assessments(f.scope(), &consent_key, &configuration.revision_id, 3)
            .await
            .is_err()
    );
    assert_eq!(f.store.admit_outcome_assessments(32).await?, 0);
    let initial = f
        .store
        .get_snapshot(&f.workspace.id, &f.project.id, &f.origin.id)
        .await?;
    for turn in initial.nonterminal_turns {
        f.store
            .cancel_turn(
                &f.workspace.id,
                &f.project.id,
                &f.origin.id,
                &turn.id,
                &key(),
            )
            .await?;
    }
    assert_eq!(
        f.store.admit_outcome_assessments(32).await?,
        0,
        "progress and missing results invoke no model"
    );
    accept_coding(&f).await?;
    assert_eq!(f.store.admit_outcome_assessments(32).await?, 1);
    assert_eq!(f.store.admit_outcome_assessments(32).await?, 0);
    let (db, driver) = connect(&f.runtime).await?;
    let rows = db
        .query(
            "SELECT turn_id,snapshot FROM workflow_outcome_assessments WHERE conversation_id=$1",
            &[&f.origin.id],
        )
        .await?;
    assert_eq!(rows.len(), 1);
    let turn: String = rows[0].get(0);
    let snapshot: Vec<u8> = rows[0].get(1);
    assert!(!String::from_utf8_lossy(&snapshot).contains("PRIVATE_"));
    let bytes = command_bytes(&db, &turn).await?;
    let reopened = WorkflowStore::new(f.runtime.clone());
    assert_eq!(reopened.admit_outcome_assessments(32).await?, 0);
    assert_eq!(command_bytes(&db, &turn).await?, bytes);
    reopened
        .cancel_turn(&f.workspace.id, &f.project.id, &f.origin.id, &turn, &key())
        .await?;
    assert_eq!(
        reopened.admit_outcome_assessments(32).await?,
        0,
        "accepted outcome snapshot is deduplicated even after cancellation"
    );
    reopened
        .authorize_outcome_assessments(f.scope(), &key(), &configuration.revision_id, 0)
        .await?;
    cancel_after_coding(&f).await?;
    assert_eq!(
        reopened.admit_outcome_assessments(32).await?,
        0,
        "revocation prevents new assessments after changed outcomes"
    );
    reopened
        .authorize_outcome_assessments(f.scope(), &key(), &configuration.revision_id, 1)
        .await?;
    let changed = ayni_orchestrator_workflow_postgres::model::AgentConfiguration {
        revision_id: String::new(),
        instructions: "Changed workspace instructions".into(),
        ..configuration.clone()
    };
    let current = reopened
        .update_agent_configuration(&key(), &configuration.revision_id, &changed)
        .await?;
    assert_eq!(
        reopened.admit_outcome_assessments(32).await?,
        0,
        "configuration changes pause old automatic consent"
    );
    let status = reopened.outcome_assessment_status(f.scope()).await?;
    assert_eq!(
        status.authorization.configuration_revision_id,
        configuration.revision_id
    );
    assert_ne!(
        status.authorization.configuration_revision_id,
        current.revision_id
    );
    assert_eq!(status.admitted_assessments, 0);
    assert!(
        reopened
            .authorize_outcome_assessments(f.scope(), &key(), &configuration.revision_id, 1)
            .await
            .is_err()
    );
    driver.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL with separate runtime role"]
async fn an_old_attempts_new_outcome_enters_the_bounded_assessment_selection() -> Result {
    let f = fixture().await?;
    let siblings = f.store.list_tasks(f.scope(), "", 32).await?.tasks;
    for task in siblings
        .iter()
        .filter(|task| task.id != f.prepared.task_id)
        .take(17)
    {
        let prepared = f
            .store
            .prepare_task_execution(f.scope(), &key(), &task.id, &f.prepared.inputs)
            .await?;
        let sibling = Fixture {
            store: f.store.clone(),
            runtime: f.runtime.clone(),
            workspace: f.workspace.clone(),
            project: f.project.clone(),
            origin: f.origin.clone(),
            prepared,
            pod_uid: key(),
        };
        accept_coding(&sibling).await?;
        cancel_after_coding(&sibling).await?;
    }
    // This attempt was prepared first, but its accepted outcome is now newest.
    accept_coding(&f).await?;
    let initial = f
        .store
        .get_snapshot(&f.workspace.id, &f.project.id, &f.origin.id)
        .await?;
    for turn in initial.nonterminal_turns {
        f.store
            .cancel_turn(
                &f.workspace.id,
                &f.project.id,
                &f.origin.id,
                &turn.id,
                &key(),
            )
            .await?;
    }
    let configuration = f.store.get_agent_configuration(&f.workspace.id, "").await?;
    f.store
        .authorize_outcome_assessments(f.scope(), &key(), &configuration.revision_id, 1)
        .await?;
    assert_eq!(f.store.admit_outcome_assessments(32).await?, 1);
    let (db, driver) = connect(&f.runtime).await?;
    let row = db
        .query_one(
            "SELECT snapshot FROM workflow_outcome_assessments WHERE conversation_id=$1",
            &[&f.origin.id],
        )
        .await?;
    let bytes: Vec<u8> = row.get(0);
    let facts: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(facts.as_array().map(Vec::len), Some(16));
    assert!(facts.as_array().is_some_and(|facts| {
        facts
            .iter()
            .any(|fact| fact["attempt_id"] == f.prepared.attempt_id)
    }));
    cancel_after_coding(&f).await?;
    driver.abort();
    Ok(())
}
