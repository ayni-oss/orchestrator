use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio_postgres::{Client, Transaction};

pub const MIGRATION_LOCK_ID: i64 = 4_702_199_126_064_921;
pub const CURRENT_SCHEMA_VERSION: i64 = 9;

const MIGRATION_1: &str = include_str!("../migrations/0001_durable_foundation.sql");
const MIGRATION_2: &str = include_str!("../migrations/0002_report_acceptance.sql");
const MIGRATION_3: &str = include_str!("../migrations/0003_transport_quarantine.sql");
const MIGRATION_4: &str = include_str!("../migrations/0004_watch_retention.sql");
const MIGRATION_5: &str = include_str!("../migrations/0005_delivery_accounting.sql");
const MIGRATION_6: &str = include_str!("../migrations/0006_transport_rejections.sql");
const MIGRATION_7: &str = include_str!("../migrations/0007_broker_capacity.sql");
const MIGRATION_8: &str = include_str!("../migrations/0008_agent_configuration.sql");
const MIGRATION_9: &str = include_str!("../migrations/0009_sessions.sql");

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("PostgreSQL migration is unavailable")]
    Database(#[from] tokio_postgres::Error),
    #[error("workflow schema version {0} is newer than this runtime")]
    FutureVersion(i64),
    #[error("workflow schema migration history is invalid")]
    InvalidHistory,
    #[error("workflow schema migration checksum does not match retained state")]
    ChecksumMismatch,
    #[error("workflow schema validation failed")]
    InvalidSchema,
    #[error("workflow runtime database role is invalid")]
    InvalidRuntimeRole,
    #[error("workflow runtime database privileges are invalid")]
    InvalidRuntimePrivileges,
}

/// Apply the exact supported workflow schema under a transaction-scoped advisory lock.
///
/// # Errors
/// Returns a database or schema-history error without modifying a future or corrupt schema.
pub async fn migrate(client: &mut Client) -> Result<(), MigrationError> {
    migrate_inner(client, None).await
}

/// Apply the schema and grant one distinct least-privilege runtime role.
///
/// # Errors
/// Returns a migration error if the role is absent, privileged, or aliases the migrator.
pub async fn migrate_for_runtime(
    client: &mut Client,
    runtime_role: &str,
) -> Result<(), MigrationError> {
    migrate_inner(client, Some(runtime_role)).await
}

async fn migrate_inner(
    client: &mut Client,
    runtime_role: Option<&str>,
) -> Result<(), MigrationError> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute("SET LOCAL search_path TO public, pg_catalog, pg_temp")
        .await?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])
        .await?;
    transaction
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS public.workflow_schema_migrations (\
                 version bigint PRIMARY KEY CHECK (version > 0),\
                 description text NOT NULL,\
                 checksum text NOT NULL,\
                 applied_at timestamptz NOT NULL DEFAULT clock_timestamp()\
             )",
        )
        .await?;
    validate_history(&transaction).await?;
    let current: i64 = transaction
        .query_one(
            "SELECT COALESCE(max(version), 0) FROM public.workflow_schema_migrations",
            &[],
        )
        .await?
        .get(0);
    if current > CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::FutureVersion(current));
    }
    if current == 0 {
        transaction.batch_execute(MIGRATION_1).await?;
        transaction
            .execute(
                "INSERT INTO public.workflow_schema_migrations (version, description, checksum) \
                 VALUES (1, $1, $2)",
                &[
                    &"M1 durable workflow foundation",
                    &migration_checksum(MIGRATION_1),
                ],
            )
            .await?;
    }
    if current <= 1 {
        transaction.batch_execute(MIGRATION_2).await?;
        transaction
            .execute(
                "INSERT INTO public.workflow_schema_migrations (version, description, checksum) \
                 VALUES (2, $1, $2)",
                &[
                    &"M1 executor report acceptance",
                    &migration_checksum(MIGRATION_2),
                ],
            )
            .await?;
    }
    for (version, source, description) in [
        (3_i64, MIGRATION_3, "M1 transport quarantine"),
        (4_i64, MIGRATION_4, "M1 watch retention"),
        (5_i64, MIGRATION_5, "M1 delivery accounting"),
        (6_i64, MIGRATION_6, "M1 detached report rejection"),
        (7_i64, MIGRATION_7, "M1 broker capacity observations"),
        (
            8_i64,
            MIGRATION_8,
            "Workspace agent configuration and frozen turn inputs",
        ),
        (9_i64, MIGRATION_9, "Isolated conversation sessions"),
    ] {
        if current < version {
            transaction.batch_execute(source).await?;
            transaction.execute("INSERT INTO public.workflow_schema_migrations (version, description, checksum) VALUES ($1,$2,$3)", &[&version,&description,&migration_checksum(source)]).await?;
        }
    }
    validate_checksum(&transaction).await?;
    validate_schema(&transaction).await?;
    if let Some(runtime_role) = runtime_role {
        apply_runtime_privileges(&transaction, runtime_role).await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Validate that the runtime connection sees the exact supported workflow schema.
///
/// # Errors
/// Returns a database, version, checksum, history, schema-shape, or privilege error.
pub async fn validate(client: &Client) -> Result<(), MigrationError> {
    validate_inner(client).await
}

/// Validate the schema and prove that the connection uses the configured runtime role.
///
/// # Errors
/// Returns a validation error if the connected role differs or has unsafe privileges.
pub async fn validate_for_runtime(
    client: &Client,
    runtime_role: &str,
) -> Result<(), MigrationError> {
    validate_role_name(runtime_role)?;
    let identities = client
        .query_one("SELECT current_user, session_user", &[])
        .await?;
    let current_role: String = identities.get(0);
    let session_role: String = identities.get(1);
    if current_role != runtime_role || session_role != runtime_role {
        return Err(MigrationError::InvalidRuntimeRole);
    }
    validate_inner(client).await
}

async fn validate_inner(client: &Client) -> Result<(), MigrationError> {
    let rows = client
        .query(
            "SELECT version, checksum FROM public.workflow_schema_migrations ORDER BY version",
            &[],
        )
        .await?;
    if rows
        .last()
        .is_some_and(|row| row.get::<_, i64>(0) > CURRENT_SCHEMA_VERSION)
    {
        return Err(MigrationError::FutureVersion(
            rows.last().ok_or(MigrationError::InvalidHistory)?.get(0),
        ));
    }
    let migrations = [
        (1_i64, MIGRATION_1),
        (2_i64, MIGRATION_2),
        (3_i64, MIGRATION_3),
        (4_i64, MIGRATION_4),
        (5_i64, MIGRATION_5),
        (6_i64, MIGRATION_6),
        (7_i64, MIGRATION_7),
        (8_i64, MIGRATION_8),
        (9_i64, MIGRATION_9),
    ];
    if rows.len() != migrations.len() {
        return Err(MigrationError::InvalidHistory);
    }
    for (row, (expected_version, migration)) in rows.iter().zip(migrations) {
        if row.get::<_, i64>(0) != expected_version {
            return Err(MigrationError::InvalidHistory);
        }
        if row.get::<_, String>(1) != migration_checksum(migration) {
            return Err(MigrationError::ChecksumMismatch);
        }
    }
    validate_schema_client(client).await?;
    validate_runtime_privileges(client).await
}

async fn validate_history(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let rows = transaction
        .query(
            "SELECT version FROM public.workflow_schema_migrations ORDER BY version",
            &[],
        )
        .await?;
    for (index, row) in rows.iter().enumerate() {
        let expected = i64::try_from(index)
            .map_err(|_| MigrationError::InvalidHistory)?
            .saturating_add(1);
        let actual: i64 = row.get(0);
        if actual != expected {
            return Err(MigrationError::InvalidHistory);
        }
    }
    Ok(())
}

async fn validate_checksum(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    for (version, migration) in [
        (1_i64, MIGRATION_1),
        (2_i64, MIGRATION_2),
        (3_i64, MIGRATION_3),
        (4_i64, MIGRATION_4),
        (5_i64, MIGRATION_5),
        (6_i64, MIGRATION_6),
        (7_i64, MIGRATION_7),
        (8_i64, MIGRATION_8),
        (9_i64, MIGRATION_9),
    ] {
        let checksum: String = transaction
            .query_one(
                "SELECT checksum FROM public.workflow_schema_migrations WHERE version = $1",
                &[&version],
            )
            .await?
            .get(0);
        if checksum != migration_checksum(migration) {
            return Err(MigrationError::ChecksumMismatch);
        }
    }
    Ok(())
}

async fn apply_runtime_privileges(
    transaction: &Transaction<'_>,
    runtime_role: &str,
) -> Result<(), MigrationError> {
    validate_role_name(runtime_role)?;
    let role = transaction
        .query_opt(
            "SELECT quote_ident(role.rolname) FROM pg_roles AS role \
             WHERE role.rolname = $1 AND role.rolcanlogin \
               AND NOT role.rolsuper AND NOT role.rolcreatedb \
               AND NOT role.rolcreaterole AND NOT role.rolreplication \
               AND NOT role.rolbypassrls AND role.rolname <> current_user \
               AND has_database_privilege(role.rolname, current_database(), 'CONNECT') \
               AND NOT has_database_privilege( \
                   role.rolname, current_database(), 'CREATE,TEMPORARY' \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_auth_members AS membership \
                   WHERE membership.member = role.oid OR membership.roleid = role.oid \
               )",
            &[&runtime_role],
        )
        .await?
        .ok_or(MigrationError::InvalidRuntimeRole)?
        .get::<_, String>(0);
    transaction
        .batch_execute(&format!(
            "ALTER DEFAULT PRIVILEGES IN SCHEMA public \
               REVOKE ALL PRIVILEGES ON TABLES FROM {role};\
             ALTER DEFAULT PRIVILEGES IN SCHEMA public \
               REVOKE ALL PRIVILEGES ON SEQUENCES FROM {role};\
             ALTER DEFAULT PRIVILEGES IN SCHEMA public \
               REVOKE ALL PRIVILEGES ON FUNCTIONS FROM {role};\
             GRANT USAGE ON SCHEMA public TO {role};\
             REVOKE ALL PRIVILEGES ON TABLE \
               workflow_workspaces, workflow_projects, workflow_conversations, \
               workflow_command_receipts, workflow_messages, workflow_turns, \
               workflow_activities, workflow_updates, workflow_dispatch_intents, \
               workflow_outbox, workflow_history_snapshots, workflow_report_receipts, workflow_transport_quarantine, workflow_watch_watermarks, workflow_pending_reports, workflow_transport_rejections, workflow_broker_capacity, workflow_agent_configurations, workflow_agent_configuration_heads, workflow_configuration_receipts, workflow_turn_inputs, workflow_sessions, workflow_turn_sessions, workflow_session_receipts, workflow_context_manifests, workflow_context_manifest_messages FROM {role};\
             GRANT SELECT, INSERT ON TABLE \
               workflow_workspaces, workflow_projects, workflow_conversations, \
               workflow_command_receipts, workflow_messages, workflow_turns, \
               workflow_activities, workflow_updates, workflow_dispatch_intents, \
               workflow_outbox, workflow_history_snapshots, workflow_report_receipts, workflow_transport_quarantine, workflow_watch_watermarks, workflow_pending_reports, workflow_transport_rejections, workflow_broker_capacity, workflow_agent_configurations, workflow_agent_configuration_heads, workflow_configuration_receipts, workflow_turn_inputs, workflow_sessions, workflow_turn_sessions, workflow_session_receipts, workflow_context_manifests, workflow_context_manifest_messages TO {role};\
             GRANT UPDATE (state,idle_timeout_seconds,last_active_at) ON workflow_sessions TO {role};\
             GRANT UPDATE (revision_id) ON workflow_agent_configuration_heads TO {role};\
             GRANT UPDATE (revision, next_turn_sequence, next_history_sequence) \
               ON workflow_conversations TO {role};\
             GRANT UPDATE (resume_cursor) ON workflow_command_receipts TO {role};\
             GRANT UPDATE (dispatch_state, execution_state, response_message_id, failure_reason, updated_at, \
               scheduling_state, context_revision, assignment_id, executor_generation) ON workflow_turns TO {role};\
             GRANT UPDATE (state, attempt_count, last_safe_reason, next_attempt_at, updated_at, stream_sequence) \
               ON workflow_outbox TO {role};\
             GRANT UPDATE (eligible, command_event_id, exact_command_bytes, updated_at) \
               ON workflow_dispatch_intents TO {role};\
             GRANT DELETE ON workflow_history_snapshots, workflow_transport_quarantine, workflow_updates, workflow_activities, workflow_pending_reports TO {role};\
             GRANT UPDATE (retained_messages,retained_bytes,observed_at) ON workflow_broker_capacity TO {role};\
             GRANT UPDATE (state,stream_sequence,attempt_count,next_attempt_at,updated_at) ON workflow_transport_rejections TO {role};\
             GRANT UPDATE (pruned_through_cursor) ON workflow_watch_watermarks TO {role};\
             REVOKE ALL PRIVILEGES ON SEQUENCE workflow_updates_cursor_seq FROM {role};\
             GRANT USAGE, SELECT ON SEQUENCE workflow_updates_cursor_seq TO {role};\
             REVOKE ALL PRIVILEGES ON FUNCTION workflow_ts_seconds(timestamptz), \
               workflow_ts_nanos(timestamptz) FROM {role};\
             GRANT EXECUTE ON FUNCTION workflow_ts_seconds(timestamptz), \
               workflow_ts_nanos(timestamptz) TO {role};\
             REVOKE ALL PRIVILEGES ON TABLE public.workflow_schema_migrations FROM {role};\
             GRANT SELECT ON TABLE public.workflow_schema_migrations TO {role};"
        ))
        .await?;
    Ok(())
}

fn validate_role_name(runtime_role: &str) -> Result<(), MigrationError> {
    if runtime_role.is_empty()
        || runtime_role.len() > 63
        || !runtime_role.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_lowercase() || (index > 0 && byte.is_ascii_digit())
        })
    {
        Err(MigrationError::InvalidRuntimeRole)
    } else {
        Ok(())
    }
}

const SCHEMA_MANIFEST_CHECKSUM: &str =
    "sha256:5b3b0f2eb910d697df9032965a9c412df972e08e3780d62dbdabd4daabe0c563";

async fn validate_schema(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let manifest: String = transaction
        .query_one(schema_manifest_query(), &[])
        .await?
        .get(0);
    validate_schema_manifest(&manifest)?;
    let safe: bool = transaction
        .query_one(schema_guards_query(), &[])
        .await?
        .get(0);
    validate_schema_guards(safe)
}

async fn validate_schema_client(client: &Client) -> Result<(), MigrationError> {
    let manifest: String = client.query_one(schema_manifest_query(), &[]).await?.get(0);
    validate_schema_manifest(&manifest)?;
    let safe: bool = client.query_one(schema_guards_query(), &[]).await?.get(0);
    validate_schema_guards(safe)
}

#[allow(
    clippy::too_many_lines,
    reason = "the exact runtime ACL expression remains contiguous for security review"
)]
async fn validate_runtime_privileges(client: &Client) -> Result<(), MigrationError> {
    let valid: bool = client
        .query_one(
            "SELECT \
               role.rolcanlogin \
               AND NOT role.rolsuper AND NOT role.rolcreatedb AND NOT role.rolcreaterole \
               AND NOT role.rolreplication AND NOT role.rolbypassrls \
               AND has_database_privilege(current_user, current_database(), 'CONNECT') \
               AND NOT has_database_privilege( \
                   current_user, current_database(), 'CREATE,TEMPORARY' \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_auth_members AS membership \
                   WHERE membership.member = role.oid OR membership.roleid = role.oid \
               ) \
               AND (SELECT database.datdba <> role.oid FROM pg_database AS database \
                    WHERE database.datname = current_database()) \
               AND (SELECT namespace.nspowner <> role.oid FROM pg_namespace AS namespace \
                    WHERE namespace.nspname = 'public') \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_namespace AS namespace \
                   CROSS JOIN LATERAL aclexplode(namespace.nspacl) AS acl \
                   WHERE namespace.nspname = 'public' \
                     AND acl.grantee = role.oid AND acl.is_grantable \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_class AS object \
                   JOIN pg_namespace AS namespace ON namespace.oid = object.relnamespace \
                   WHERE namespace.nspname = 'public' \
                     AND object.relname LIKE 'workflow\\_%' ESCAPE '\\' \
                     AND object.relowner = role.oid \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_proc AS function \
                   JOIN pg_namespace AS namespace ON namespace.oid = function.pronamespace \
                   WHERE namespace.nspname = 'public' \
                     AND function.proname LIKE 'workflow\\_%' ESCAPE '\\' \
                     AND function.proowner = role.oid \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM information_schema.table_privileges AS privilege \
                   WHERE privilege.grantee = current_user \
                     AND privilege.table_schema = 'public' \
                     AND privilege.table_name LIKE 'workflow\\_%' ESCAPE '\\' \
                     AND privilege.is_grantable = 'YES' \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM information_schema.routine_privileges AS privilege \
                   WHERE privilege.grantee = current_user \
                     AND privilege.specific_schema = 'public' \
                     AND privilege.routine_name LIKE 'workflow\\_%' ESCAPE '\\' \
                     AND privilege.is_grantable = 'YES' \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM information_schema.usage_privileges AS privilege \
                   WHERE privilege.grantee = current_user \
                     AND privilege.object_schema = 'public' \
                     AND (privilege.object_name = 'public' \
                          OR privilege.object_name LIKE 'workflow\\_%' ESCAPE '\\') \
                     AND privilege.is_grantable = 'YES' \
               ) \
               AND has_schema_privilege(current_user, 'public', 'USAGE') \
               AND NOT has_schema_privilege(current_user, 'public', 'CREATE') \
               AND has_table_privilege( \
                   current_user, 'public.workflow_schema_migrations', 'SELECT' \
               ) \
               AND NOT has_table_privilege( \
                   current_user, 'public.workflow_schema_migrations', \
                   'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER' \
               ) \
               AND ( \
                   SELECT bool_and( \
                       has_table_privilege(current_user, table_name, 'SELECT') \
                       AND has_table_privilege(current_user, table_name, 'INSERT') \
                       AND NOT has_table_privilege( \
                           current_user, table_name, 'UPDATE,TRUNCATE,REFERENCES,TRIGGER' \
                       ) \
                       AND has_table_privilege(current_user, table_name, 'DELETE') \
                           = (table_name IN ('public.workflow_history_snapshots','public.workflow_transport_quarantine','public.workflow_updates','public.workflow_activities','public.workflow_pending_reports')) \
                   ) \
                   FROM unnest(ARRAY[ \
                       'public.workflow_workspaces', 'public.workflow_projects', \
                       'public.workflow_conversations', 'public.workflow_command_receipts', \
                       'public.workflow_messages', 'public.workflow_turns', \
                       'public.workflow_activities', 'public.workflow_updates', \
                       'public.workflow_dispatch_intents', 'public.workflow_outbox', \
                       'public.workflow_history_snapshots', 'public.workflow_report_receipts', \
                       'public.workflow_transport_quarantine', 'public.workflow_watch_watermarks', 'public.workflow_pending_reports', 'public.workflow_transport_rejections', 'public.workflow_broker_capacity', 'public.workflow_agent_configurations', 'public.workflow_agent_configuration_heads', 'public.workflow_configuration_receipts', 'public.workflow_turn_inputs', 'public.workflow_sessions', 'public.workflow_turn_sessions', 'public.workflow_session_receipts', 'public.workflow_context_manifests', 'public.workflow_context_manifest_messages' \
                   ]) AS table_name \
               ) \
               AND ( \
                   SELECT bool_and(has_column_privilege( \
                       current_user, format('public.%I', required.table_name), \
                       required.column_name, 'UPDATE' \
                   )) \
                   FROM (VALUES \
                       ('workflow_sessions','state'),('workflow_sessions','idle_timeout_seconds'),('workflow_sessions','last_active_at'), \
                       ('workflow_agent_configuration_heads','revision_id'), \
                       ('workflow_broker_capacity','retained_messages'),('workflow_broker_capacity','retained_bytes'),('workflow_broker_capacity','observed_at'), \
                       ('workflow_transport_rejections','state'),('workflow_transport_rejections','stream_sequence'),('workflow_transport_rejections','attempt_count'),('workflow_transport_rejections','next_attempt_at'),('workflow_transport_rejections','updated_at'), \
                       ('workflow_watch_watermarks', 'pruned_through_cursor'), \
                       ('workflow_conversations', 'revision'), \
                       ('workflow_conversations', 'next_turn_sequence'), \
                       ('workflow_conversations', 'next_history_sequence'), \
                       ('workflow_command_receipts', 'resume_cursor'), \
                       ('workflow_turns', 'dispatch_state'), \
                       ('workflow_turns', 'execution_state'), \
                       ('workflow_turns', 'response_message_id'), \
                       ('workflow_turns', 'failure_reason'), \
                       ('workflow_turns', 'updated_at'), \
                       ('workflow_turns', 'scheduling_state'), \
                       ('workflow_turns', 'context_revision'), \
                       ('workflow_turns', 'assignment_id'), \
                       ('workflow_turns', 'executor_generation'), \
                       ('workflow_outbox', 'state'), \
                       ('workflow_outbox', 'attempt_count'), \
                       ('workflow_outbox', 'last_safe_reason'), \
                       ('workflow_outbox', 'next_attempt_at'), \
                       ('workflow_outbox', 'updated_at'), \
                       ('workflow_outbox', 'stream_sequence'), \
                       ('workflow_dispatch_intents', 'eligible'), \
                       ('workflow_dispatch_intents', 'command_event_id'), \
                       ('workflow_dispatch_intents', 'exact_command_bytes'), \
                       ('workflow_dispatch_intents', 'updated_at') \
                   ) AS required(table_name, column_name) \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM information_schema.columns AS candidate \
                   WHERE candidate.table_schema = 'public' \
                     AND candidate.table_name LIKE 'workflow\\_%' ESCAPE '\\' \
                     AND has_column_privilege( \
                         current_user, \
                         format('public.%I', candidate.table_name), \
                         candidate.column_name, 'UPDATE' \
                     ) \
                     AND NOT ( \
                         (candidate.table_name='workflow_sessions' AND candidate.column_name IN ('state','idle_timeout_seconds','last_active_at')) \
                         OR (candidate.table_name='workflow_agent_configuration_heads' AND candidate.column_name='revision_id') \
                         OR (candidate.table_name='workflow_broker_capacity' AND candidate.column_name IN ('retained_messages','retained_bytes','observed_at')) \
                         OR (candidate.table_name='workflow_transport_rejections' AND candidate.column_name IN ('state','stream_sequence','attempt_count','next_attempt_at','updated_at')) \
                         OR (candidate.table_name = 'workflow_watch_watermarks' AND candidate.column_name = 'pruned_through_cursor') \
                         OR (candidate.table_name = 'workflow_conversations' \
                          AND candidate.column_name IN ( \
                              'revision', 'next_turn_sequence', 'next_history_sequence' \
                          )) \
                         OR (candidate.table_name = 'workflow_command_receipts' \
                             AND candidate.column_name = 'resume_cursor') \
                         OR (candidate.table_name = 'workflow_turns' \
                             AND candidate.column_name IN ('dispatch_state', 'execution_state', \
                               'response_message_id', 'failure_reason', 'updated_at', 'scheduling_state', \
                               'context_revision', 'assignment_id', 'executor_generation')) \
                         OR (candidate.table_name = 'workflow_outbox' \
                             AND candidate.column_name IN ('state', 'attempt_count', 'last_safe_reason', \
                               'next_attempt_at', 'updated_at', 'stream_sequence')) \
                         OR (candidate.table_name = 'workflow_dispatch_intents' \
                             AND candidate.column_name IN ( \
                               'eligible', 'command_event_id', 'exact_command_bytes', 'updated_at' \
                             )) \
                     ) \
               ) \
               AND has_sequence_privilege( \
                   current_user, 'public.workflow_updates_cursor_seq', 'USAGE' \
               ) \
               AND has_sequence_privilege( \
                   current_user, 'public.workflow_updates_cursor_seq', 'SELECT' \
               ) \
               AND NOT has_sequence_privilege( \
                   current_user, 'public.workflow_updates_cursor_seq', 'UPDATE' \
               ) \
               AND has_function_privilege( \
                   current_user, 'public.workflow_ts_seconds(timestamptz)', 'EXECUTE' \
               ) \
               AND has_function_privilege( \
                   current_user, 'public.workflow_ts_nanos(timestamptz)', 'EXECUTE' \
               ) \
             FROM pg_roles AS role WHERE role.rolname = current_user",
            &[],
        )
        .await?
        .get(0);
    if valid {
        Ok(())
    } else {
        Err(MigrationError::InvalidRuntimePrivileges)
    }
}

const fn validate_schema_guards(safe: bool) -> Result<(), MigrationError> {
    if safe {
        Ok(())
    } else {
        Err(MigrationError::InvalidSchema)
    }
}

const fn schema_guards_query() -> &'static str {
    r"
SELECT
    NOT EXISTS (
        SELECT 1
        FROM pg_class AS relation
        JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
        WHERE namespace.nspname = 'public'
          AND relation.relname LIKE 'workflow\_%' ESCAPE '\'
          AND relation.relkind IN ('r', 'i', 'S')
          AND (
              relation.relpersistence <> 'p'
              OR relation.relrowsecurity
              OR relation.relforcerowsecurity
          )
    )
    AND NOT EXISTS (
        SELECT 1
        FROM pg_trigger AS trigger
        JOIN pg_class AS relation ON relation.oid = trigger.tgrelid
        JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
        WHERE namespace.nspname = 'public'
          AND relation.relname LIKE 'workflow\_%' ESCAPE '\'
          AND NOT trigger.tgisinternal
    )
    AND NOT EXISTS (
        SELECT 1
        FROM pg_policy AS policy
        JOIN pg_class AS relation ON relation.oid = policy.polrelid
        JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
        WHERE namespace.nspname = 'public'
          AND relation.relname LIKE 'workflow\_%' ESCAPE '\'
    )
    AND NOT EXISTS (
        SELECT 1
        FROM pg_rewrite AS rule
        JOIN pg_class AS relation ON relation.oid = rule.ev_class
        JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
        WHERE namespace.nspname = 'public'
          AND relation.relname LIKE 'workflow\_%' ESCAPE '\'
          AND rule.rulename <> '_RETURN'
    )
"
}

fn validate_schema_manifest(manifest: &str) -> Result<(), MigrationError> {
    if migration_checksum(manifest) == SCHEMA_MANIFEST_CHECKSUM {
        Ok(())
    } else {
        Err(MigrationError::InvalidSchema)
    }
}

const fn schema_manifest_query() -> &'static str {
    r"
WITH relations AS (
    SELECT class.oid, class.relname, class.relkind
    FROM pg_class AS class
    JOIN pg_namespace AS namespace ON namespace.oid = class.relnamespace
    WHERE namespace.nspname = 'public'
      AND class.relname LIKE 'workflow\_%' ESCAPE '\'
      AND class.relkind IN ('r', 'i', 'S')
), entries(kind, identity, definition) AS (
    SELECT 'relation', relname, relkind::text
    FROM relations
    UNION ALL
    SELECT
        'column',
        relation.relname || '.' || attribute.attnum::text || '.' || attribute.attname,
        concat_ws(
            '|',
            format_type(attribute.atttypid, attribute.atttypmod),
            attribute.attnotnull::text,
            attribute.attidentity,
            attribute.attgenerated,
            COALESCE(pg_get_expr(default_value.adbin, default_value.adrelid), '')
        )
    FROM relations AS relation
    JOIN pg_attribute AS attribute ON attribute.attrelid = relation.oid
    LEFT JOIN pg_attrdef AS default_value
      ON default_value.adrelid = relation.oid AND default_value.adnum = attribute.attnum
    WHERE relation.relkind = 'r'
      AND attribute.attnum > 0
      AND NOT attribute.attisdropped
    UNION ALL
    SELECT
        'constraint',
        relation.relname || '.' || constraint_value.conname,
        concat_ws(
            '|',
            constraint_value.contype::text,
            constraint_value.condeferrable::text,
            constraint_value.condeferred::text,
            constraint_value.convalidated::text,
            pg_get_constraintdef(constraint_value.oid, true)
        )
    FROM relations AS relation
    JOIN pg_constraint AS constraint_value ON constraint_value.conrelid = relation.oid
    WHERE relation.relkind = 'r'
    UNION ALL
    SELECT 'index', relation.relname, pg_get_indexdef(relation.oid)
    FROM relations AS relation
    WHERE relation.relkind = 'i'
    UNION ALL
    SELECT
        'function',
        procedure.proname || '(' || pg_get_function_identity_arguments(procedure.oid) || ')',
        concat_ws(
            '|',
            pg_get_function_result(procedure.oid),
            procedure.provolatile::text,
            procedure.proparallel::text,
            procedure.prokind::text,
            pg_get_functiondef(procedure.oid)
        )
    FROM pg_proc AS procedure
    JOIN pg_namespace AS namespace ON namespace.oid = procedure.pronamespace
    WHERE namespace.nspname = 'public'
      AND procedure.proname LIKE 'workflow\_%' ESCAPE '\'
)
SELECT COALESCE(
    string_agg(
        kind || chr(31) || identity || chr(31) || definition,
        chr(30) ORDER BY kind, identity, definition
    ),
    ''
)
FROM entries
"
}

fn migration_checksum(source: &str) -> String {
    let digest = Sha256::digest(source.as_bytes());
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[must_use]
pub const fn migration_lock_id() -> i64 {
    MIGRATION_LOCK_ID
}
