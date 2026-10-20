use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio_postgres::{Client, Transaction};

pub const MIGRATION_LOCK_ID: i64 = 4_702_199_126_064_921;
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

const MIGRATION_1: &str = include_str!("../migrations/0001_durable_foundation.sql");

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
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])
        .await?;
    transaction
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS workflow_schema_migrations (\
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
            "SELECT COALESCE(max(version), 0) FROM workflow_schema_migrations",
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
                "INSERT INTO workflow_schema_migrations (version, description, checksum) \
                 VALUES ($1, $2, $3)",
                &[
                    &CURRENT_SCHEMA_VERSION,
                    &"M1 durable workflow foundation",
                    &migration_checksum(MIGRATION_1),
                ],
            )
            .await?;
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
    let connected_role: String = client.query_one("SELECT current_user", &[]).await?.get(0);
    if connected_role != runtime_role {
        return Err(MigrationError::InvalidRuntimeRole);
    }
    validate_inner(client).await
}

async fn validate_inner(client: &Client) -> Result<(), MigrationError> {
    let row = client
        .query_opt(
            "SELECT version, checksum FROM workflow_schema_migrations ORDER BY version DESC LIMIT 1",
            &[],
        )
        .await?
        .ok_or(MigrationError::InvalidHistory)?;
    let version: i64 = row.get(0);
    if version > CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::FutureVersion(version));
    }
    if version != CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::InvalidHistory);
    }
    let checksum: String = row.get(1);
    if checksum != migration_checksum(MIGRATION_1) {
        return Err(MigrationError::ChecksumMismatch);
    }
    validate_schema_client(client).await?;
    validate_runtime_privileges(client).await
}

async fn validate_history(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let rows = transaction
        .query(
            "SELECT version FROM workflow_schema_migrations ORDER BY version",
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
    let checksum: String = transaction
        .query_one(
            "SELECT checksum FROM workflow_schema_migrations WHERE version = $1",
            &[&CURRENT_SCHEMA_VERSION],
        )
        .await?
        .get(0);
    if checksum != migration_checksum(MIGRATION_1) {
        return Err(MigrationError::ChecksumMismatch);
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
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_auth_members AS membership \
                   WHERE membership.member = role.oid \
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
               workflow_outbox, workflow_history_snapshots FROM {role};\
             GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE \
               workflow_workspaces, workflow_projects, workflow_conversations, \
               workflow_command_receipts, workflow_messages, workflow_turns, \
               workflow_activities, workflow_updates, workflow_dispatch_intents, \
               workflow_outbox, workflow_history_snapshots TO {role};\
             REVOKE ALL PRIVILEGES ON SEQUENCE workflow_updates_cursor_seq FROM {role};\
             GRANT USAGE, SELECT ON SEQUENCE workflow_updates_cursor_seq TO {role};\
             REVOKE ALL PRIVILEGES ON FUNCTION workflow_ts_seconds(timestamptz), \
               workflow_ts_nanos(timestamptz) FROM {role};\
             GRANT EXECUTE ON FUNCTION workflow_ts_seconds(timestamptz), \
               workflow_ts_nanos(timestamptz) TO {role};\
             REVOKE ALL PRIVILEGES ON TABLE workflow_schema_migrations FROM {role};\
             GRANT SELECT ON TABLE workflow_schema_migrations TO {role};"
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
    "sha256:b917c86f2811ac54235c62a2799ae9821d3f96dd28e6f444a2ce7c2edec74ef8";

async fn validate_schema(transaction: &Transaction<'_>) -> Result<(), MigrationError> {
    let manifest: String = transaction
        .query_one(schema_manifest_query(), &[])
        .await?
        .get(0);
    validate_schema_manifest(&manifest)
}

async fn validate_schema_client(client: &Client) -> Result<(), MigrationError> {
    let manifest: String = client.query_one(schema_manifest_query(), &[]).await?.get(0);
    validate_schema_manifest(&manifest)
}

async fn validate_runtime_privileges(client: &Client) -> Result<(), MigrationError> {
    let valid: bool = client
        .query_one(
            "SELECT \
               role.rolcanlogin \
               AND NOT role.rolsuper AND NOT role.rolcreatedb AND NOT role.rolcreaterole \
               AND NOT role.rolreplication AND NOT role.rolbypassrls \
               AND NOT EXISTS ( \
                   SELECT 1 FROM pg_auth_members AS membership \
                   WHERE membership.member = role.oid \
               ) \
               AND has_schema_privilege(current_user, 'public', 'USAGE') \
               AND NOT has_schema_privilege(current_user, 'public', 'CREATE') \
               AND has_table_privilege(current_user, 'workflow_schema_migrations', 'SELECT') \
               AND NOT has_table_privilege( \
                   current_user, 'workflow_schema_migrations', \
                   'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER' \
               ) \
               AND ( \
                   SELECT bool_and( \
                       has_table_privilege(current_user, table_name, 'SELECT') \
                       AND has_table_privilege(current_user, table_name, 'INSERT') \
                       AND has_table_privilege(current_user, table_name, 'UPDATE') \
                       AND has_table_privilege(current_user, table_name, 'DELETE') \
                       AND NOT has_table_privilege( \
                           current_user, table_name, 'TRUNCATE,REFERENCES,TRIGGER' \
                       ) \
                   ) \
                   FROM unnest(ARRAY[ \
                       'workflow_workspaces', 'workflow_projects', \
                       'workflow_conversations', 'workflow_command_receipts', \
                       'workflow_messages', 'workflow_turns', 'workflow_activities', \
                       'workflow_updates', 'workflow_dispatch_intents', 'workflow_outbox', \
                       'workflow_history_snapshots' \
                   ]) AS table_name \
               ) \
               AND has_sequence_privilege( \
                   current_user, 'workflow_updates_cursor_seq', 'USAGE' \
               ) \
               AND has_sequence_privilege( \
                   current_user, 'workflow_updates_cursor_seq', 'SELECT' \
               ) \
               AND NOT has_sequence_privilege( \
                   current_user, 'workflow_updates_cursor_seq', 'UPDATE' \
               ) \
               AND has_function_privilege( \
                   current_user, 'workflow_ts_seconds(timestamptz)', 'EXECUTE' \
               ) \
               AND has_function_privilege( \
                   current_user, 'workflow_ts_nanos(timestamptz)', 'EXECUTE' \
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
