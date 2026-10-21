use ayni_orchestrator_events::{
    EventIdentity,
    v1::{ConversationTurnBinding, ConversationTurnReport, ConversationTurnReportKind},
    workflow::{
        WorkflowEventMetadata, decode_command, decode_report, encode_report, sign_report,
        signing_key_digest,
    },
};
use ayni_orchestrator_workflow_postgres::{
    CURRENT_SCHEMA_VERSION, MigrationError, StoreError, WorkflowStore, connect, migrate,
    migrate_for_runtime, migration_lock_id, validate, validate_client_for_runtime,
    validate_for_runtime,
};
use ed25519_dalek::SigningKey;
use sha2::{Digest as _, Sha256};
use std::{error::Error, sync::Arc, time::Duration};
use tokio::{
    sync::Barrier,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_postgres::Client;
use uuid::Uuid;

const DATABASE_URL_ENV: &str = "AYNI_POSTGRES_FEASIBILITY_URL";
const RUNTIME_DATABASE_URL_ENV: &str = "AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL";
const RUNTIME_DATABASE_ROLE_ENV: &str = "AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE";
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires a disposable PostgreSQL 17 instance"]
async fn postgres_feasibility_gate() -> TestResult {
    let database_url = std::env::var(DATABASE_URL_ENV)
        .map_err(|_| format!("set {DATABASE_URL_ENV} to a disposable PostgreSQL database"))?;
    let runtime_database_url = std::env::var(RUNTIME_DATABASE_URL_ENV).map_err(|_| {
        format!("set {RUNTIME_DATABASE_URL_ENV} to the disposable runtime-role database URL")
    })?;
    let runtime_database_role = std::env::var(RUNTIME_DATABASE_ROLE_ENV)
        .map_err(|_| format!("set {RUNTIME_DATABASE_ROLE_ENV} to the runtime role name"))?;
    let (client, driver) = connect(&database_url).await?;
    reset(&client).await?;
    prove_advisory_lock(&client, &database_url).await?;
    reset(&client).await?;
    prove_migration_failure_rollback(&client, &database_url).await?;
    prove_concurrent_migrations(&client, &database_url).await?;
    migrate_for_runtime(&database_url, &runtime_database_role).await?;
    validate(&runtime_database_url).await?;
    prove_safe_search_path(&runtime_database_url).await?;
    prove_runtime_role(&runtime_database_url).await?;
    prove_schema_drift_rejected(&client, &runtime_database_url).await?;
    prove_runtime_privilege_drift_rejected(
        &client,
        &database_url,
        &runtime_database_url,
        &runtime_database_role,
    )
    .await?;
    prove_transaction_rollback(&client).await?;
    prove_admission_insert_rollbacks(&client, &runtime_database_url).await?;
    prove_authoritative_store(&client, &runtime_database_url).await?;
    prove_future_version_rejected(&client, &runtime_database_url).await?;
    driver.abort();
    println!(
        "postgres-runtime: advisory-lock=pass migration=pass migration-rollback=pass schema-drift=pass role-separation=pass privilege-drift=pass transaction-rollback=pass admission-rollback=pass scope=pass idempotency=pass fifo=pass queue-bound=pass exact-outbox=pass report-acceptance=pass fifo-promotion=pass snapshot-history=pass future-version=pass"
    );
    Ok(())
}

async fn prove_advisory_lock(client: &Client, database_url: &str) -> TestResult {
    client
        .query_one("SELECT pg_advisory_lock($1)", &[&migration_lock_id()])
        .await?;
    let application_name = format!("ayni_m1_migration_lock_probe_{}", std::process::id());
    let migration_url = database_url_with_application_name(database_url, &application_name);
    let blocked = tokio::spawn(async move { migrate(&migration_url).await });
    wait_for_advisory_lock_wait(client, &application_name, migration_lock_id(), &blocked).await?;
    assert!(!blocked.is_finished());
    let schema_visible: bool = client
        .query_one(
            "SELECT to_regclass('public.workflow_schema_migrations') IS NOT NULL",
            &[],
        )
        .await?
        .get(0);
    assert!(!schema_visible);
    client
        .query_one("SELECT pg_advisory_unlock($1)", &[&migration_lock_id()])
        .await?;
    blocked.await??;
    Ok(())
}

async fn prove_migration_failure_rollback(client: &Client, database_url: &str) -> TestResult {
    client
        .batch_execute("CREATE TABLE public.workflow_projects (injected boolean)")
        .await?;
    assert!(matches!(
        migrate(database_url).await,
        Err(MigrationError::Database(_))
    ));
    let state = client
        .query_one(
            "SELECT \
               to_regclass('public.workflow_projects') IS NOT NULL, \
               to_regclass('public.workflow_schema_migrations') IS NULL, \
               to_regclass('public.workflow_workspaces') IS NULL, \
               to_regprocedure('public.workflow_ts_seconds(timestamptz)') IS NULL",
            &[],
        )
        .await?;
    assert!(state.get::<_, bool>(0));
    assert!(state.get::<_, bool>(1));
    assert!(state.get::<_, bool>(2));
    assert!(state.get::<_, bool>(3));
    reset(client).await
}

async fn prove_concurrent_migrations(client: &Client, database_url: &str) -> TestResult {
    let count = 8;
    let barrier = Arc::new(Barrier::new(count));
    let mut tasks = Vec::new();
    for _ in 0..count {
        let url = database_url.to_owned();
        let barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            migrate(&url).await
        }));
    }
    for task in tasks {
        task.await??;
    }
    let versions: i64 = client
        .query_one("SELECT count(*) FROM workflow_schema_migrations", &[])
        .await?
        .get(0);
    assert_eq!(versions, CURRENT_SCHEMA_VERSION);
    Ok(())
}

async fn prove_safe_search_path(database_url: &str) -> TestResult {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    let hostile_url = format!(
        "{database_url}{separator}options=-c%20search_path%3Dpublic%2Cpg_temp%2Cpg_catalog"
    );
    let (client, driver) = connect(&hostile_url).await?;
    let search_path: String = client.query_one("SHOW search_path", &[]).await?.get(0);
    assert_eq!(search_path, "pg_catalog,public,pg_temp");
    driver.abort();
    Ok(())
}

async fn prove_runtime_role(database_url: &str) -> TestResult {
    let (client, driver) = connect(database_url).await?;
    let privileges = client
        .query_one(
            "SELECT \
               has_table_privilege(current_user, 'workflow_workspaces', 'SELECT'), \
               has_table_privilege(current_user, 'workflow_workspaces', 'INSERT'), \
               has_table_privilege(current_user, 'workflow_workspaces', 'UPDATE,DELETE'), \
               has_column_privilege( \
                   current_user, 'workflow_conversations', 'revision', 'UPDATE' \
               ), \
               has_table_privilege(current_user, 'workflow_history_snapshots', 'DELETE'), \
               has_table_privilege(current_user, 'workflow_schema_migrations', 'SELECT'), \
               has_table_privilege(current_user, 'workflow_schema_migrations', 'INSERT,UPDATE,DELETE'), \
               has_schema_privilege(current_user, 'public', 'CREATE'), \
               has_database_privilege(current_user, current_database(), 'CONNECT'), \
               has_database_privilege(current_user, current_database(), 'CREATE,TEMPORARY')",
            &[],
        )
        .await?;
    assert!(privileges.get::<_, bool>(0));
    assert!(privileges.get::<_, bool>(1));
    assert!(!privileges.get::<_, bool>(2));
    assert!(privileges.get::<_, bool>(3));
    assert!(privileges.get::<_, bool>(4));
    assert!(privileges.get::<_, bool>(5));
    assert!(!privileges.get::<_, bool>(6));
    assert!(!privileges.get::<_, bool>(7));
    assert!(privileges.get::<_, bool>(8));
    assert!(!privileges.get::<_, bool>(9));
    assert!(
        client
            .execute(
                "DELETE FROM workflow_schema_migrations WHERE version = 1",
                &[],
            )
            .await
            .is_err()
    );
    assert!(
        client
            .execute("UPDATE workflow_messages SET content = 'tampered'", &[])
            .await
            .is_err()
    );
    driver.abort();
    Ok(())
}

async fn prove_schema_drift_rejected(client: &Client, runtime_url: &str) -> TestResult {
    client
        .batch_execute("DROP FUNCTION workflow_ts_nanos(timestamptz)")
        .await?;
    assert!(matches!(
        validate(runtime_url).await,
        Err(MigrationError::InvalidSchema)
    ));
    client
        .batch_execute(
            "CREATE FUNCTION public.workflow_ts_nanos(value timestamptz) \
             RETURNS bigint LANGUAGE sql IMMUTABLE PARALLEL SAFE \
             RETURN (extract(microseconds FROM value)::bigint % 1000000) * 1000",
        )
        .await?;
    validate(runtime_url).await?;

    client
        .batch_execute(
            "CREATE FUNCTION public.ayni_schema_drift() RETURNS trigger LANGUAGE plpgsql \
             AS $drift$ BEGIN RETURN NEW; END; $drift$; \
             CREATE TRIGGER ayni_schema_drift BEFORE INSERT ON workflow_workspaces \
             FOR EACH ROW EXECUTE FUNCTION ayni_schema_drift()",
        )
        .await?;
    assert!(matches!(
        validate(runtime_url).await,
        Err(MigrationError::InvalidSchema)
    ));
    client
        .batch_execute(
            "DROP TRIGGER ayni_schema_drift ON workflow_workspaces; \
             DROP FUNCTION ayni_schema_drift()",
        )
        .await?;

    client
        .batch_execute("ALTER TABLE workflow_schema_migrations SET UNLOGGED")
        .await?;
    assert!(matches!(
        validate(runtime_url).await,
        Err(MigrationError::InvalidSchema)
    ));
    client
        .batch_execute("ALTER TABLE workflow_schema_migrations SET LOGGED")
        .await?;
    client
        .batch_execute("ALTER TABLE workflow_workspaces ENABLE ROW LEVEL SECURITY")
        .await?;
    assert!(matches!(
        validate(runtime_url).await,
        Err(MigrationError::InvalidSchema)
    ));
    client
        .batch_execute("ALTER TABLE workflow_workspaces DISABLE ROW LEVEL SECURITY")
        .await?;
    validate(runtime_url).await?;
    Ok(())
}

async fn prove_runtime_privilege_drift_rejected(
    client: &Client,
    migration_url: &str,
    runtime_url: &str,
    runtime_role: &str,
) -> TestResult {
    let quoted_role: String = client
        .query_one("SELECT quote_ident($1)", &[&runtime_role])
        .await?
        .get(0);
    client
        .batch_execute(&format!(
            "REVOKE INSERT ON workflow_workspaces FROM {quoted_role}"
        ))
        .await?;
    assert!(matches!(
        validate(runtime_url).await,
        Err(MigrationError::InvalidRuntimePrivileges)
    ));
    migrate_for_runtime(migration_url, runtime_role).await?;
    validate_for_runtime(runtime_url, runtime_role).await?;

    client
        .batch_execute(&format!(
            "REVOKE UPDATE (dispatch_state) ON workflow_turns FROM {quoted_role}"
        ))
        .await?;
    assert!(matches!(
        validate(runtime_url).await,
        Err(MigrationError::InvalidRuntimePrivileges)
    ));
    migrate_for_runtime(migration_url, runtime_role).await?;
    validate_for_runtime(runtime_url, runtime_role).await?;

    client
        .batch_execute(&format!(
            "GRANT TRUNCATE ON workflow_workspaces TO {quoted_role}"
        ))
        .await?;
    assert!(matches!(
        validate_for_runtime(runtime_url, runtime_role).await,
        Err(MigrationError::InvalidRuntimePrivileges)
    ));
    migrate_for_runtime(migration_url, runtime_role).await?;
    validate_for_runtime(runtime_url, runtime_role).await?;

    client
        .batch_execute(&format!(
            "GRANT UPDATE ON workflow_messages TO {quoted_role}"
        ))
        .await?;
    assert!(matches!(
        validate_for_runtime(runtime_url, runtime_role).await,
        Err(MigrationError::InvalidRuntimePrivileges)
    ));
    migrate_for_runtime(migration_url, runtime_role).await?;
    validate_for_runtime(runtime_url, runtime_role).await?;

    client
        .batch_execute(&format!(
            "GRANT SELECT ON workflow_workspaces TO {quoted_role} WITH GRANT OPTION"
        ))
        .await?;
    assert!(matches!(
        validate_for_runtime(runtime_url, runtime_role).await,
        Err(MigrationError::InvalidRuntimePrivileges)
    ));
    migrate_for_runtime(migration_url, runtime_role).await?;
    validate_for_runtime(runtime_url, runtime_role).await?;

    client
        .batch_execute(&format!(
            "CREATE ROLE ayni_runtime_inheritor; \
             GRANT {quoted_role} TO ayni_runtime_inheritor"
        ))
        .await?;
    assert!(matches!(
        validate_for_runtime(runtime_url, runtime_role).await,
        Err(MigrationError::InvalidRuntimePrivileges)
    ));
    client
        .batch_execute(&format!(
            "REVOKE {quoted_role} FROM ayni_runtime_inheritor; \
             DROP ROLE ayni_runtime_inheritor; \
             SET ROLE {quoted_role}"
        ))
        .await?;
    assert!(matches!(
        validate_client_for_runtime(client, runtime_role).await,
        Err(MigrationError::InvalidRuntimeRole)
    ));
    client.batch_execute("RESET ROLE").await?;
    validate_for_runtime(runtime_url, runtime_role).await?;
    assert!(matches!(
        validate_for_runtime(runtime_url, "different_runtime").await,
        Err(MigrationError::InvalidRuntimeRole)
    ));
    Ok(())
}

async fn prove_transaction_rollback(client: &Client) -> TestResult {
    client.batch_execute("BEGIN").await?;
    client
        .execute(
            "INSERT INTO workflow_workspaces (id, display_name) VALUES ($1, $2)",
            &[&"018f3f2e-72b7-7d4d-8000-000000000001", &"rolled back"],
        )
        .await?;
    client.batch_execute("ROLLBACK").await?;
    let retained: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_workspaces WHERE display_name = 'rolled back'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(retained, 0);
    Ok(())
}

async fn prove_admission_insert_rollbacks(client: &Client, database_url: &str) -> TestResult {
    let store = WorkflowStore::new(database_url.to_owned());
    let workspace = store
        .create_workspace("rollback-workspace", "Rollback Workspace")
        .await?;
    let project = store
        .create_project(
            "rollback-project",
            &workspace.resource.id,
            "Rollback Project",
            "Prove admission rollback",
        )
        .await?;
    let conversation = store
        .create_conversation(
            "rollback-conversation",
            &workspace.resource.id,
            &project.resource.id,
            "Rollback Conversation",
        )
        .await?;
    client
        .batch_execute(
            "CREATE FUNCTION public.ayni_abort_admission() RETURNS trigger \
             LANGUAGE plpgsql AS $abort$ BEGIN RAISE EXCEPTION 'injected admission failure'; END; $abort$",
        )
        .await?;
    let baseline = admission_state(client, &conversation.resource.id).await?;
    let insert_points = [
        ("workflow_command_receipts", "INSERT"),
        ("workflow_conversations", "UPDATE"),
        ("workflow_messages", "INSERT"),
        ("workflow_turns", "INSERT"),
        ("workflow_activities", "INSERT"),
        ("workflow_updates", "INSERT"),
        ("workflow_dispatch_intents", "INSERT"),
        ("workflow_outbox", "INSERT"),
    ];
    for (index, (table, operation)) in insert_points.iter().enumerate() {
        client
            .batch_execute(&format!(
                "CREATE TRIGGER ayni_abort_point BEFORE {operation} ON {table} \
                 FOR EACH ROW EXECUTE FUNCTION ayni_abort_admission()"
            ))
            .await?;
        let result = store
            .send_message(
                &format!("rollback-message-{index}"),
                &workspace.resource.id,
                &project.resource.id,
                &conversation.resource.id,
                "must roll back",
            )
            .await;
        client
            .batch_execute(&format!("DROP TRIGGER ayni_abort_point ON {table}"))
            .await?;
        assert!(matches!(result, Err(StoreError::Database(_))));
        assert_eq!(
            admission_state(client, &conversation.resource.id).await?,
            baseline
        );
    }
    client
        .batch_execute("DROP FUNCTION ayni_abort_admission()")
        .await?;
    Ok(())
}

async fn admission_state(client: &Client, conversation_id: &str) -> TestResult<Vec<i64>> {
    let row = client
        .query_one(
            "SELECT revision, next_turn_sequence, next_history_sequence, \
               (SELECT count(*) FROM workflow_command_receipts), \
               (SELECT count(*) FROM workflow_messages WHERE conversation_id = $1), \
               (SELECT count(*) FROM workflow_turns WHERE conversation_id = $1), \
               (SELECT count(*) FROM workflow_activities WHERE conversation_id = $1), \
               (SELECT count(*) FROM workflow_updates WHERE conversation_id = $1), \
               (SELECT count(*) FROM workflow_dispatch_intents WHERE conversation_id = $1), \
               (SELECT count(*) FROM workflow_outbox WHERE conversation_id = $1) \
             FROM workflow_conversations WHERE id = $1",
            &[&conversation_id],
        )
        .await?;
    Ok((0..row.len()).map(|index| row.get(index)).collect())
}

#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end test keeps the authoritative transaction invariants in execution order"
)]
async fn prove_authoritative_store(client: &Client, database_url: &str) -> TestResult {
    let store = WorkflowStore::new(database_url.to_owned());
    let workspace = store.create_workspace("workspace-key", "Workspace").await?;
    let workspace_replay = store.create_workspace("workspace-key", "Workspace").await?;
    assert_eq!(workspace, workspace_replay);
    assert!(matches!(
        store.create_workspace("workspace-key", "Changed").await,
        Err(StoreError::IdempotencyConflict)
    ));
    let project = store
        .create_project(
            "project-key",
            &workspace.resource.id,
            "Project",
            "Durable workflow",
        )
        .await?;
    let foreign_workspace = store.create_workspace("foreign-key", "Foreign").await?;
    assert!(matches!(
        store
            .create_conversation(
                "wrong-scope",
                &foreign_workspace.resource.id,
                &project.resource.id,
                "Hidden",
            )
            .await,
        Err(StoreError::NotFound)
    ));
    let conversation = store
        .create_conversation(
            "conversation-key",
            &workspace.resource.id,
            &project.resource.id,
            "Conversation",
        )
        .await?;
    let first = store
        .send_message(
            "same-message",
            &workspace.resource.id,
            &project.resource.id,
            &conversation.resource.id,
            "first",
        )
        .await?;
    let first_replay = store
        .send_message(
            "same-message",
            &workspace.resource.id,
            &project.resource.id,
            &conversation.resource.id,
            "first",
        )
        .await?;
    assert_eq!(first, first_replay);
    assert!(matches!(
        store
            .send_message(
                "same-message",
                &workspace.resource.id,
                &project.resource.id,
                &conversation.resource.id,
                "changed",
            )
            .await,
        Err(StoreError::IdempotencyConflict)
    ));

    prove_concurrent_receipt_replay(
        &store,
        &workspace.resource.id,
        &project.resource.id,
        &conversation.resource.id,
    )
    .await?;
    admit_concurrent_messages(
        &store,
        &workspace.resource.id,
        &project.resource.id,
        &conversation.resource.id,
        15,
    )
    .await?;
    let rows = client
        .query(
            "SELECT sequence, scheduling_state, dispatch_state FROM workflow_turns \
             WHERE conversation_id = $1 ORDER BY sequence",
            &[&conversation.resource.id],
        )
        .await?;
    assert_eq!(rows.len(), 17);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.get::<_, i64>(0), i64::try_from(index)? + 1);
        if index == 0 {
            assert_eq!(row.get::<_, &str>(1), "eligible");
            assert_eq!(row.get::<_, &str>(2), "pending");
        } else {
            assert_eq!(row.get::<_, &str>(1), "waiting_for_predecessor");
            assert_eq!(row.get::<_, &str>(2), "not_eligible");
        }
    }
    let outbox = store.pending_outbox(10).await?;
    assert_eq!(outbox.len(), 1);
    let (_, _, command) = decode_command(&outbox[0].event_bytes)?;
    let binding = command.binding.ok_or("command binding is absent")?;
    assert_eq!(binding.turn_id, first.turn_id);
    assert_eq!(command.input_content, "first");

    let snapshot = store
        .get_snapshot(
            &workspace.resource.id,
            &project.resource.id,
            &conversation.resource.id,
        )
        .await?;
    assert_eq!(snapshot.nonterminal_turns.len(), 17);
    assert_eq!(snapshot.recent_messages.len(), 16);
    let history = store
        .list_history(
            &workspace.resource.id,
            &project.resource.id,
            &conversation.resource.id,
            &snapshot.history_snapshot_id,
            0,
            10,
        )
        .await?;
    assert_eq!(history.records.len(), 10);
    assert!(history.has_more);
    client
        .execute(
            "INSERT INTO workflow_history_snapshots \
             (id, workspace_id, project_id, conversation_id, boundary_sequence, expires_at) \
             VALUES ('expired-snapshot', $1, $2, $3, 0, clock_timestamp() - interval '1 second')",
            &[
                &workspace.resource.id,
                &project.resource.id,
                &conversation.resource.id,
            ],
        )
        .await?;
    store
        .get_snapshot(
            &workspace.resource.id,
            &project.resource.id,
            &conversation.resource.id,
        )
        .await?;
    let expired: i64 = client
        .query_one(
            "SELECT count(*) FROM workflow_history_snapshots WHERE id = 'expired-snapshot'",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(expired, 0);

    prove_report_acceptance(client, &store, &workspace.resource.id, &project.resource.id).await?;
    prove_capacity_race(
        client,
        &store,
        &workspace.resource.id,
        &project.resource.id,
        &conversation.resource.id,
    )
    .await?;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "the report handoff scenario keeps claim, terminal, and FIFO evidence in causal order"
)]
async fn prove_report_acceptance(
    client: &Client,
    store: &WorkflowStore,
    workspace_id: &str,
    project_id: &str,
) -> TestResult {
    let conversation = store
        .create_conversation(
            "report-conversation",
            workspace_id,
            project_id,
            "Report acceptance",
        )
        .await?;
    let first = store
        .send_message(
            "report-first",
            workspace_id,
            project_id,
            &conversation.resource.id,
            "first scripted input",
        )
        .await?;
    let second = store
        .send_message(
            "report-second",
            workspace_id,
            project_id,
            &conversation.resource.id,
            "second scripted input",
        )
        .await?;
    store
        .send_message(
            "report-third",
            workspace_id,
            project_id,
            &conversation.resource.id,
            "third queued input",
        )
        .await?;
    let first_outbox = store
        .pending_outbox(1_000)
        .await?
        .into_iter()
        .find(|record| record.turn_id == first.turn_id)
        .ok_or("first command is absent")?;
    let (command_identity, metadata, command) = decode_command(&first_outbox.event_bytes)?;
    let binding = command.binding.ok_or("first command binding is absent")?;
    let winner = Uuid::now_v7().to_string();
    let winner_seed = [7_u8; 32];
    let claim = signed_report(
        &binding,
        &metadata,
        &first_outbox.event_bytes,
        &winner,
        &winner_seed,
        ConversationTurnReportKind::Started,
        "",
    )?;
    let attacker = Uuid::now_v7().to_string();
    let invalid_claim = signed_report(
        &binding,
        &metadata,
        &first_outbox.event_bytes,
        &attacker,
        &[5_u8; 32],
        ConversationTurnReportKind::Started,
        "",
    )?;
    let (invalid_identity, invalid_metadata, mut invalid_payload) = decode_report(&invalid_claim)?;
    *invalid_payload
        .signature
        .first_mut()
        .ok_or("claim signature is absent")? ^= 1;
    let invalid_claim = encode_report(&invalid_identity, &invalid_metadata, &invalid_payload)?;
    let invalid_receipt = store.accept_claim(&invalid_claim).await?;
    assert!(!invalid_receipt.accepted);
    assert_eq!(invalid_receipt.safe_reason, "invalid_signature");

    let claim_receipt = store.accept_claim(&claim).await?;
    assert!(claim_receipt.accepted);
    assert_eq!(store.accept_claim(&claim).await?, claim_receipt);

    let loser = Uuid::now_v7().to_string();
    let losing_claim = signed_report(
        &binding,
        &metadata,
        &first_outbox.event_bytes,
        &loser,
        &[9_u8; 32],
        ConversationTurnReportKind::Started,
        "",
    )?;
    let losing_receipt = store.accept_claim(&losing_claim).await?;
    assert!(!losing_receipt.accepted);
    assert_eq!(losing_receipt.safe_reason, "claim_lost");

    let terminal = signed_report(
        &binding,
        &metadata,
        &first_outbox.event_bytes,
        &winner,
        &winner_seed,
        ConversationTurnReportKind::Completed,
        "scripted response",
    )?;
    let invalid_terminal = signed_report(
        &binding,
        &metadata,
        &first_outbox.event_bytes,
        &winner,
        &winner_seed,
        ConversationTurnReportKind::Completed,
        "untrusted response",
    )?;
    let (invalid_identity, invalid_metadata, mut invalid_payload) =
        decode_report(&invalid_terminal)?;
    *invalid_payload
        .signature
        .first_mut()
        .ok_or("terminal signature is absent")? ^= 1;
    let invalid_terminal = encode_report(&invalid_identity, &invalid_metadata, &invalid_payload)?;
    let invalid_receipt = store.accept_terminal_report(&invalid_terminal).await?;
    assert!(!invalid_receipt.accepted);
    assert_eq!(invalid_receipt.safe_reason, "invalid_signature");

    let terminal_receipt = store.accept_terminal_report(&terminal).await?;
    assert!(terminal_receipt.accepted);
    assert_eq!(
        store.accept_terminal_report(&terminal).await?,
        terminal_receipt
    );

    let snapshot = store
        .get_snapshot(workspace_id, project_id, &conversation.resource.id)
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
            &[&first.turn_id],
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
                .is_some_and(|candidate| candidate.turn_id == second.turn_id)
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
    assert_eq!(binding.command_event_id, command_identity.id);
    Ok(())
}

fn signed_report(
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
    };
    sign_report(&identity, &mut report, seed)?;
    Ok(encode_report(&identity, metadata, &report)?)
}

async fn prove_concurrent_receipt_replay(
    store: &WorkflowStore,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
) -> TestResult {
    let callers = 8;
    let barrier = Arc::new(Barrier::new(callers));
    let mut sends = Vec::new();
    for _ in 0..callers {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        let workspace_id = workspace_id.to_owned();
        let project_id = project_id.to_owned();
        let conversation_id = conversation_id.to_owned();
        sends.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .send_message(
                    "concurrent-replay",
                    &workspace_id,
                    &project_id,
                    &conversation_id,
                    "same concurrent content",
                )
                .await
        }));
    }
    let mut receipts = Vec::new();
    for send in sends {
        receipts.push(send.await??);
    }
    assert!(receipts.windows(2).all(|pair| pair[0] == pair[1]));
    Ok(())
}

async fn admit_concurrent_messages(
    store: &WorkflowStore,
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
    count: usize,
) -> TestResult {
    let barrier = Arc::new(Barrier::new(count));
    let mut sends = Vec::new();
    for index in 0..count {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        let workspace_id = workspace_id.to_owned();
        let project_id = project_id.to_owned();
        let conversation_id = conversation_id.to_owned();
        sends.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .send_message(
                    &format!("message-{index}"),
                    &workspace_id,
                    &project_id,
                    &conversation_id,
                    &format!("content-{index}"),
                )
                .await
        }));
    }
    for send in sends {
        send.await??;
    }
    Ok(())
}

async fn prove_capacity_race(
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

async fn prove_future_version_rejected(client: &Client, database_url: &str) -> TestResult {
    client
        .execute(
            "INSERT INTO workflow_schema_migrations (version, description, checksum) \
             VALUES (3, 'future', 'sha256:future')",
            &[],
        )
        .await?;
    assert!(matches!(
        validate(database_url).await,
        Err(MigrationError::FutureVersion(3))
    ));
    client
        .execute(
            "DELETE FROM workflow_schema_migrations WHERE version = 3",
            &[],
        )
        .await?;
    Ok(())
}

async fn reset(client: &Client) -> TestResult {
    client
        .batch_execute(
            "DROP SCHEMA public CASCADE; CREATE SCHEMA public; \
             REVOKE CREATE ON SCHEMA public FROM PUBLIC; \
             GRANT USAGE ON SCHEMA public TO PUBLIC;",
        )
        .await?;
    Ok(())
}

fn database_url_with_application_name(database_url: &str, application_name: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}application_name={application_name}")
}

async fn wait_for_advisory_lock_wait<T>(
    client: &Client,
    application_name: &str,
    lock_id: i64,
    migration: &JoinHandle<T>,
) -> TestResult<i32> {
    timeout(Duration::from_secs(5), async {
        loop {
            if migration.is_finished() {
                return Err("migration completed before waiting on the advisory lock".into());
            }
            let waiting = client
                .query_opt(
                    "SELECT activity.pid \
                     FROM pg_stat_activity activity \
                     JOIN pg_locks lock ON lock.pid = activity.pid \
                     WHERE activity.application_name = $1 \
                       AND lock.locktype = 'advisory' \
                       AND NOT lock.granted \
                       AND lock.classid::bigint = (($2::bigint >> 32) & 4294967295) \
                       AND lock.objid::bigint = ($2::bigint & 4294967295) \
                       AND lock.objsubid = 1",
                    &[&application_name, &lock_id],
                )
                .await?;
            if let Some(row) = waiting {
                return Ok(row.get(0));
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "migration did not reach the expected advisory-lock wait within five seconds")?
}
