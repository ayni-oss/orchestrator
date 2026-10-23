//! Publication review, immutable inputs, action permits and restart fencing.
use ayni_orchestrator_events::{
    task_execution::{
        SupervisorRequest as TaskSupervisor, TaskOutcome, WorkerRequest as TaskWorker, digest,
    },
    task_publication::{
        PublicationOutcome, PublicationSupervisorRequest as Supervisor,
        PublicationWorkerRequest as Worker,
    },
    task_quality::{QualityOutcome, snapshot_digest},
};
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore, connect,
    execution::{ExecutionInputs, TaskExecutionPreparation},
    migrate_for_runtime,
    plans::{PlanContent, PlanScope, PlanTaskSpecification},
};
use std::error::Error;
use uuid::Uuid;
type Result<T = ()> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const REPOSITORY: &str = "https://github.com/example/publication-fixture.git";
fn key() -> String {
    Uuid::now_v7().to_string()
}
struct Fixture {
    store: WorkflowStore,
    admin: String,
    runtime: String,
    workspace: String,
    project: String,
    conversation: String,
    attempts: Vec<TaskExecutionPreparation>,
    quality: QualityOutcome,
}
impl Fixture {
    fn scope(&self) -> PlanScope<'_> {
        PlanScope {
            workspace: &self.workspace,
            project: &self.project,
            conversation: &self.conversation,
        }
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL with separate runtime role"]
async fn publication_freezes_reviewed_evidence_and_never_reissues_create_authority() -> Result {
    let f = fixture().await?;
    deny_ineligible_and_rollback(&f).await?;
    let a = &f.attempts[0];
    let patch = digest(b"diff --git a/a b/a\n");
    let quality = digest(&serde_json::to_vec(&f.quality)?);
    let request_key = key();
    let receipt = f
        .store
        .start_task_publication(
            f.scope(),
            &a.task_id,
            &a.attempt_id,
            &request_key,
            &patch,
            &quality,
        )
        .await?;
    assert_eq!(receipt.publication_id, a.attempt_id);
    assert_eq!(
        f.store
            .start_task_publication(
                f.scope(),
                &a.task_id,
                &a.attempt_id,
                &request_key,
                &patch,
                &quality
            )
            .await?,
        receipt
    );
    assert!(matches!(
        f.store
            .start_task_publication(
                f.scope(),
                &a.task_id,
                &a.attempt_id,
                &request_key,
                &digest(b"changed"),
                &quality
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert!(
        f.store
            .start_task_publication(
                f.scope(),
                &a.task_id,
                &a.attempt_id,
                &key(),
                &patch,
                &quality
            )
            .await
            .is_err()
    );
    let (uid, commit) = claim_and_consume(&f, &receipt.publication_id, &patch, &quality).await?;
    recover_ambiguous_create(&f, &receipt.publication_id, &uid, &commit).await?;
    cancel_and_expire(&f).await?;
    println!(
        "publication-authority: reviewed-quality=pass immutable-inputs=pass receipt-retry=pass rollback=pass consumed-create-permit=pass lease-fence=pass uncertain-recovery=pass cancellation=pass expiry=pass"
    );
    Ok(())
}
async fn fixture() -> Result<Fixture> {
    let admin = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?;
    let runtime = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
    migrate_for_runtime(
        &admin,
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?,
    )
    .await?;
    let store = WorkflowStore::new(runtime.clone());
    let workspace = store
        .create_workspace(&key(), "Publication")
        .await?
        .resource
        .id;
    let project = store
        .create_project(&key(), &workspace, "Publication", "Drafts")
        .await?
        .resource
        .id;
    let conversation = store
        .create_conversation(&key(), &workspace, &project, "Origin")
        .await?
        .resource
        .id;
    let scope = PlanScope {
        workspace: &workspace,
        project: &project,
        conversation: &conversation,
    };
    let content = PlanContent {
        title: "Reviewable changes".into(),
        objective: "Publish reviewed changes".into(),
        scope: "Only selected repository".into(),
        tasks: (0..4)
            .map(|n| PlanTaskSpecification {
                key: format!("task{n}"),
                objective: "Improve the fixture".into(),
                scope: "A single file".into(),
                acceptance_criteria: vec!["Managed quality passes".into()],
                repository: REPOSITORY.into(),
                source_ref: "refs/heads/alpha".into(),
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
    let inputs = ExecutionInputs {
        repository: REPOSITORY.into(),
        source_ref: "refs/heads/alpha".into(),
        source_commit: "a".repeat(40),
        lock_digest: digest(b"lock"),
        runtime_image: format!("example/runtime@{}", digest(b"image")),
        max_seconds: 600,
        max_tokens: 10000,
    };
    let mut attempts = Vec::new();
    let mut quality = QualityOutcome::default();
    for (n, task) in approved.tasks.iter().enumerate() {
        let a = store
            .prepare_task_execution(scope, &key(), &task.id, &inputs)
            .await?;
        let q = complete(&store, scope, &a, n != 1).await?;
        if n == 0 {
            quality = q;
        }
        attempts.push(a);
    }
    Ok(Fixture {
        store,
        admin,
        runtime,
        workspace,
        project,
        conversation,
        attempts,
        quality,
    })
}
async fn complete(
    store: &WorkflowStore,
    scope: PlanScope<'_>,
    a: &TaskExecutionPreparation,
    passing: bool,
) -> Result<QualityOutcome> {
    store
        .start_task_execution(scope, &a.task_id, &a.attempt_id, &key(), b"fixture-pack")
        .await?;
    let run = store
        .task_supervisor(&TaskSupervisor::Poll)
        .await?
        .ok_or("coding claim")?;
    let uid = key();
    store
        .task_supervisor(&TaskSupervisor::Observe {
            attempt_id: a.attempt_id.clone(),
            generation: 1,
            pod_uid: uid.clone(),
        })
        .await?;
    let outcome = TaskOutcome {
        state: "completed".into(),
        patch: "diff --git a/a b/a\n".into(),
        ..Default::default()
    };
    store
        .task_worker(
            &a.attempt_id,
            1,
            &uid,
            &TaskWorker::Report {
                outcome: outcome.clone(),
            },
        )
        .await?;
    let quality = quality(&run.command, &outcome, passing);
    assert!(quality.validate());
    store
        .task_worker(
            &a.attempt_id,
            1,
            &uid,
            &TaskWorker::QualityReport {
                outcome: quality.clone(),
            },
        )
        .await?;
    store
        .task_supervisor(&TaskSupervisor::Stopped {
            attempt_id: a.attempt_id.clone(),
            generation: 1,
            pod_uid: uid,
        })
        .await?;
    Ok(quality)
}
fn quality(
    command: &ayni_orchestrator_events::task_execution::TaskCommand,
    coding: &TaskOutcome,
    passing: bool,
) -> QualityOutcome {
    let hash = digest(b"evidence");
    let artifact=serde_json::json!({"schema_version":"0.5.0","invocation":{"command":"check","scope":{}},"execution_mode":"managed","environment_lock_fingerprint":hash,"source_fingerprint":hash,"contract_digest":hash,"prebuilt_runtime":{"metadata_digest":hash,"certificate":{"certificate":{"platform":"linux/arm64","lock_fingerprint":hash}}},"tool_versions":[{"tool":"rust","version":"1.94.0"}],"completion":{"scope":"repository","state":"complete","expected_targets":1,"completed_targets":1,"detected_targets":1,"skipped_targets":0,"issues":[]},"aggregate":{"status":if passing{"pass"}else{"fail"},"total_rows":1,"passing_rows":u64::from(passing),"failing_rows":u64::from(!passing)},"rows":[{"pass":passing}]}).to_string();
    QualityOutcome {
        state: if passing { "passed" } else { "failed" }.into(),
        safe_reason: if passing { "" } else { "quality_failed" }.into(),
        snapshot_digest: snapshot_digest(command, coding),
        validation_input_digest: hash.clone(),
        lock_digest: command.lock_digest.clone(),
        runtime_image: command.runtime_image.clone(),
        runtime_platform: "linux/arm64".into(),
        runtime_metadata_digest: hash.clone(),
        executor_digest: hash.clone(),
        contract_digest: hash.clone(),
        lock_fingerprint: hash.clone(),
        source_fingerprint: hash,
        exit_code: Some(i32::from(!passing)),
        artifact_digest: digest(artifact.as_bytes()),
        artifact,
        expected_targets: 1,
        completed_targets: 1,
    }
}
async fn deny_ineligible_and_rollback(f: &Fixture) -> Result {
    let a = &f.attempts[0];
    let bad = &f.attempts[1];
    let patch = digest(b"diff --git a/a b/a\n");
    let quality = digest(&serde_json::to_vec(&f.quality)?);
    assert!(
        f.store
            .start_task_publication(
                f.scope(),
                &bad.task_id,
                &bad.attempt_id,
                &key(),
                &patch,
                &quality
            )
            .await
            .is_err()
    );
    assert!(
        f.store
            .start_task_publication(
                f.scope(),
                &a.task_id,
                &a.attempt_id,
                &key(),
                &digest(b"changed"),
                &quality
            )
            .await
            .is_err()
    );
    assert!(
        f.store
            .start_task_publication(
                f.scope(),
                &a.task_id,
                &a.attempt_id,
                &key(),
                &patch,
                &digest(b"changed")
            )
            .await
            .is_err()
    );
    let wrong = PlanScope {
        conversation: &f.project,
        ..f.scope()
    };
    assert!(
        f.store
            .start_task_publication(wrong, &a.task_id, &a.attempt_id, &key(), &patch, &quality)
            .await
            .is_err()
    );
    let (db, driver) = connect(&f.admin).await?;
    db.batch_execute("CREATE FUNCTION fixture_reject_publication() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture'; END $$; CREATE TRIGGER fixture_reject_publication BEFORE INSERT ON workflow_task_publication_receipts FOR EACH ROW EXECUTE FUNCTION fixture_reject_publication()").await?;
    let result = f
        .store
        .start_task_publication(
            f.scope(),
            &a.task_id,
            &a.attempt_id,
            &key(),
            &patch,
            &quality,
        )
        .await;
    db.batch_execute("DROP TRIGGER fixture_reject_publication ON workflow_task_publication_receipts; DROP FUNCTION fixture_reject_publication()").await?;
    assert!(matches!(result, Err(StoreError::Database(_))));
    assert!(
        f.store
            .get_task_publication(f.scope(), &a.task_id, &a.attempt_id)
            .await?
            .is_none()
    );
    driver.abort();
    Ok(())
}
async fn recover_ambiguous_create(f: &Fixture, id: &str, old_uid: &str, commit: &str) -> Result {
    let blocked = Worker::Report {
        outcome: PublicationOutcome {
            state: "blocked".into(),
            safe_reason: "incorrect_terminal".into(),
            commit_sha: commit.into(),
            ..Default::default()
        },
    };
    assert!(
        f.store
            .publication_worker((&f.workspace, REPOSITORY), id, 1, old_uid, &blocked)
            .await
            .is_err()
    );

    assert!(
        f.store
            .cancel_task_publication(
                f.scope(),
                &f.attempts[0].task_id,
                &f.attempts[0].attempt_id,
                &key()
            )
            .await
            .is_err()
    );
    let (db, driver) = connect(&f.admin).await?;
    db.execute("UPDATE workflow_task_publications SET lease_until=clock_timestamp()-interval '1 second' WHERE publication_id=$1",&[&id]).await?;
    let uid = key();
    let poll = Supervisor::Poll {
        workspace_id: f.workspace.clone(),
        repository: REPOSITORY.into(),
        pod_uid: uid.clone(),
    };
    let run = f
        .store
        .publication_supervisor(&f.workspace, REPOSITORY, &poll)
        .await?
        .ok_or("recovered claim")?;
    assert_eq!(run.generation, 2);
    assert_eq!(run.state, "uncertain");
    assert!(
        f.store
            .publication_worker(
                (&f.workspace, REPOSITORY),
                id,
                1,
                old_uid,
                &Worker::BeginCreate
            )
            .await
            .is_err()
    );
    assert!(
        !f.store
            .publication_worker(
                (&f.workspace, REPOSITORY),
                id,
                2,
                &uid,
                &Worker::BeginCreate
            )
            .await?
            .authorized
    );
    let uncertain = PublicationOutcome {
        state: "uncertain".into(),
        safe_reason: "create_response_lost".into(),
        commit_sha: commit.into(),
        ..Default::default()
    };
    f.store
        .publication_worker(
            (&f.workspace, REPOSITORY),
            id,
            2,
            &uid,
            &Worker::Report { outcome: uncertain },
        )
        .await?;
    let outcome = PublicationOutcome {
        state: "draft".into(),
        commit_sha: commit.into(),
        pr_number: 7,
        pr_url: "https://github.com/example/publication-fixture/pull/7".into(),
        pr_state: "draft".into(),
        ..Default::default()
    };
    let report = Worker::Report {
        outcome: outcome.clone(),
    };
    f.store
        .publication_worker((&f.workspace, REPOSITORY), id, 2, &uid, &report)
        .await?;
    verify_retained_outcome(f, id, &uid, &report, &outcome).await?;
    driver.abort();
    Ok(())
}
async fn cancel_and_expire(f: &Fixture) -> Result {
    let a = &f.attempts[2];
    let run = f
        .store
        .get_task_run(f.scope(), &a.task_id, &a.attempt_id)
        .await?;
    let q = digest(&serde_json::to_vec(&run.quality.ok_or("quality")?)?);
    let patch = digest(b"diff --git a/a b/a\n");
    f.store
        .start_task_publication(f.scope(), &a.task_id, &a.attempt_id, &key(), &patch, &q)
        .await?;
    let key = key();
    let cancelled = f
        .store
        .cancel_task_publication(f.scope(), &a.task_id, &a.attempt_id, &key)
        .await?;
    assert_eq!(
        f.store
            .cancel_task_publication(f.scope(), &a.task_id, &a.attempt_id, &key)
            .await?,
        cancelled
    );
    let a = &f.attempts[3];
    let run = f
        .store
        .get_task_run(f.scope(), &a.task_id, &a.attempt_id)
        .await?;
    let q = digest(&serde_json::to_vec(&run.quality.ok_or("quality")?)?);
    f.store
        .start_task_publication(
            f.scope(),
            &a.task_id,
            &a.attempt_id,
            &Uuid::now_v7().to_string(),
            &patch,
            &q,
        )
        .await?;
    let (db, driver) = connect(&f.admin).await?;
    let old: Vec<u8> = db
        .query_one(
            "SELECT command FROM workflow_task_publications WHERE attempt_id=$1",
            &[&a.attempt_id],
        )
        .await?
        .get(0);
    let mut command: ayni_orchestrator_events::task_publication::PublicationCommand =
        serde_json::from_slice(&old)?;
    command.deadline_seconds = 1;
    db.execute(
        "UPDATE workflow_task_publications SET command=$2 WHERE attempt_id=$1",
        &[&a.attempt_id, &serde_json::to_vec(&command)?],
    )
    .await?;
    let poll = Supervisor::Poll {
        workspace_id: f.workspace.clone(),
        repository: REPOSITORY.into(),
        pod_uid: Uuid::now_v7().to_string(),
    };
    assert!(
        f.store
            .publication_supervisor(&f.workspace, REPOSITORY, &poll)
            .await?
            .is_none()
    );
    assert_eq!(
        f.store
            .get_task_publication(f.scope(), &a.task_id, &a.attempt_id)
            .await?
            .ok_or("expired")?
            .state,
        "blocked"
    );
    driver.abort();
    Ok(())
}

async fn claim_and_consume(
    f: &Fixture,
    id: &str,
    patch: &str,
    quality: &str,
) -> Result<(String, String)> {
    let uid = key();
    let poll = Supervisor::Poll {
        workspace_id: f.workspace.clone(),
        repository: REPOSITORY.into(),
        pod_uid: uid.clone(),
    };
    let claimed = f
        .store
        .publication_supervisor(&f.workspace, REPOSITORY, &poll)
        .await?
        .ok_or("claim missing")?;
    assert_eq!(claimed.command.quality_digest, quality);
    assert_eq!(claimed.command.patch_digest, patch);
    assert_eq!(
        claimed.command.branch,
        format!("task/{}", f.attempts[0].attempt_id)
    );
    assert_eq!(claimed.command.base_branch, "alpha");
    assert_eq!(claimed.command.title, "feat(tasks): Improve the fixture");
    assert_eq!(claimed.command.author_name, "Ayni Orchestrator");
    assert_eq!(claimed.command.author_email, "orchestrator@ayni.build");
    assert_eq!(
        claimed.command.commit_message,
        "feat(tasks): Improve the fixture\n\nSigned-off-by: Ayni Orchestrator <orchestrator@ayni.build>\nai-generated: true\n"
    );
    assert!(claimed.command.body.contains("Managed quality passes"));
    assert!(claimed.command.body.ends_with(&format!(
        "<!-- ayni-publication:{} -->",
        f.attempts[0].attempt_id
    )));
    assert!(claimed.command.body.contains("Acceptance criteria"));
    assert!(!claimed.command.body.contains("instructions"));
    let response = f
        .store
        .publication_worker((&f.workspace, REPOSITORY), id, 1, &uid, &Worker::Input)
        .await?;
    assert_eq!(response.source_pack, b"fixture-pack");
    let commit = "a".repeat(40);
    let tree = "b".repeat(40);
    let prepared = Worker::Prepared {
        commit_sha: commit.clone(),
        tree_sha: tree.clone(),
    };
    assert!(
        f.store
            .publication_worker((&f.workspace, REPOSITORY), id, 1, &uid, &prepared)
            .await?
            .authorized
    );
    assert!(
        f.store
            .publication_worker((&f.workspace, REPOSITORY), id, 1, &uid, &prepared)
            .await?
            .authorized
    );
    assert!(
        f.store
            .publication_worker(
                (&f.workspace, REPOSITORY),
                id,
                1,
                &key(),
                &Worker::BeginCreate
            )
            .await
            .is_err()
    );
    assert!(
        f.store
            .publication_worker(
                (&f.workspace, REPOSITORY),
                id,
                1,
                &uid,
                &Worker::BeginCreate
            )
            .await?
            .authorized
    );
    assert!(
        !f.store
            .publication_worker(
                (&f.workspace, REPOSITORY),
                id,
                1,
                &uid,
                &Worker::BeginCreate
            )
            .await?
            .authorized
    );
    Ok((uid, commit))
}

async fn verify_retained_outcome(
    f: &Fixture,
    id: &str,
    uid: &str,
    report: &Worker,
    outcome: &PublicationOutcome,
) -> Result {
    let reopened = WorkflowStore::new(f.runtime.clone());
    assert!(
        reopened
            .publication_worker((&f.workspace, REPOSITORY), id, 2, uid, report)
            .await?
            .authorized
    );
    let retained = reopened
        .get_task_publication(f.scope(), &f.attempts[0].task_id, &f.attempts[0].attempt_id)
        .await?
        .ok_or("retained")?;
    assert_eq!(retained.outcome, Some(outcome.clone()));
    let (runtime, runtime_driver) = connect(&f.runtime).await?;
    assert!(
        runtime
            .execute(
                "UPDATE workflow_task_publications SET command=command WHERE publication_id=$1",
                &[&id]
            )
            .await
            .is_err()
    );
    let history=runtime.query("SELECT ordinal,outcome FROM workflow_task_publication_reports WHERE publication_id=$1 ORDER BY ordinal",&[&id]).await?;
    assert_eq!(history.len(), 2);
    let uncertain: PublicationOutcome = serde_json::from_slice(&history[0].get::<_, Vec<u8>>(1))?;
    assert_eq!(uncertain.state, "uncertain");
    assert_eq!(uncertain.safe_reason, "create_response_lost");
    assert_eq!(
        serde_json::from_slice::<PublicationOutcome>(&history[1].get::<_, Vec<u8>>(1))?,
        *outcome
    );
    assert!(runtime.execute("UPDATE workflow_task_publication_reports SET outcome=outcome WHERE publication_id=$1",&[&id]).await.is_err());
    assert!(
        runtime
            .execute(
                "DELETE FROM workflow_task_publication_reports WHERE publication_id=$1",
                &[&id]
            )
            .await
            .is_err()
    );
    runtime_driver.abort();
    Ok(())
}
