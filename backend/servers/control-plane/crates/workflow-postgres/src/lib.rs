//! `PostgreSQL` workflow feasibility adapter.
//!
//! This crate intentionally starts with a narrow migration primitive. Runtime
//! authority is added only after the repository's `PostgreSQL` feasibility gate is
//! retained and reviewed.

use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, Error, NoTls};

const MIGRATION_LOCK_ID: i64 = 4_702_199_126_064_921;
const MIGRATION_VERSION: i64 = 1;

const MIGRATION: &str = r"
CREATE TABLE IF NOT EXISTS workflow_schema_migrations (
    version bigint PRIMARY KEY,
    description text NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE IF NOT EXISTS workflow_gate_conversations (
    id text PRIMARY KEY,
    next_sequence bigint NOT NULL DEFAULT 1 CHECK (next_sequence > 0)
);

CREATE TABLE IF NOT EXISTS workflow_gate_turns (
    id text PRIMARY KEY,
    conversation_id text NOT NULL REFERENCES workflow_gate_conversations(id) ON DELETE CASCADE,
    sequence bigint NOT NULL CHECK (sequence > 0),
    UNIQUE (conversation_id, sequence)
);

CREATE TABLE IF NOT EXISTS workflow_gate_events (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    conversation_id text NOT NULL,
    kind text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE IF NOT EXISTS workflow_gate_outbox (
    id text PRIMARY KEY,
    subject text NOT NULL,
    event_bytes bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE IF NOT EXISTS workflow_watch_conversations (
    id text PRIMARY KEY,
    title text NOT NULL,
    revision bigint NOT NULL DEFAULT 0 CHECK (revision >= 0)
);

CREATE TABLE IF NOT EXISTS workflow_watch_updates (
    cursor bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    conversation_id text NOT NULL REFERENCES workflow_watch_conversations(id) ON DELETE CASCADE,
    revision bigint NOT NULL CHECK (revision > 0),
    kind text NOT NULL,
    safe_payload text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (conversation_id, revision)
);

CREATE INDEX IF NOT EXISTS workflow_watch_updates_replay
    ON workflow_watch_updates (conversation_id, cursor);

CREATE TABLE IF NOT EXISTS workflow_watch_watermarks (
    conversation_id text PRIMARY KEY REFERENCES workflow_watch_conversations(id) ON DELETE CASCADE,
    pruned_through_cursor bigint NOT NULL DEFAULT 0 CHECK (pruned_through_cursor >= 0)
);
";

/// Open one plaintext `PostgreSQL` connection for local feasibility and return the
/// connection driver task. Production TLS and pooling are deliberately not implied.
///
/// # Errors
/// Returns the `PostgreSQL` configuration or connection error.
pub async fn connect(database_url: &str) -> Result<(Client, JoinHandle<()>), Error> {
    let config: Config = database_url.parse()?;
    let (client, connection) = config.connect(NoTls).await?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok((client, driver))
}

/// Apply the feasibility schema under a transaction-scoped advisory lock.
/// Concurrent callers serialize before inspecting and recording the version.
///
/// # Errors
/// Returns any `PostgreSQL` connection, DDL, or transaction error.
pub async fn migrate(database_url: &str) -> Result<(), Error> {
    let (mut client, driver) = connect(database_url).await?;
    let transaction = client.transaction().await?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])
        .await?;
    transaction.batch_execute(MIGRATION).await?;
    transaction
        .execute(
            "INSERT INTO workflow_schema_migrations (version, description) \
             VALUES ($1, 'workflow feasibility schema') ON CONFLICT (version) DO NOTHING",
            &[&MIGRATION_VERSION],
        )
        .await?;
    transaction.commit().await?;
    driver.abort();
    Ok(())
}

/// Stable advisory lock ID used by the reproducible feasibility test.
#[must_use]
pub const fn migration_lock_id() -> i64 {
    MIGRATION_LOCK_ID
}

/// Feasibility-only generated snapshot/watch protocol.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod watch_v1 {
    tonic::include_proto!("ayni.orchestrator.feasibility.v1");
}

pub mod watch;
