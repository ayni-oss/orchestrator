//! `PostgreSQL` authority for the replacement project-conversation workflow.
//!
//! This crate owns forward schema migrations and transaction boundaries for M1.
//! `JetStream` delivery is outside Slice 1; gRPC policy remains in the control-plane adapter.

use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, NoTls};

pub mod migrations;
pub mod model;
pub mod store;

pub use migrations::{
    CURRENT_SCHEMA_VERSION, MigrationError, migrate as migrate_client,
    migrate_for_runtime as migrate_client_for_runtime, migration_lock_id,
    validate as validate_client, validate_for_runtime as validate_client_for_runtime,
};
pub use store::{StoreError, WorkflowStore};

/// Open one plaintext `PostgreSQL` connection for the isolated local profile.
/// Production callers must select and configure an appropriate TLS connector.
///
/// # Errors
/// Returns the `PostgreSQL` configuration or connection error.
pub async fn connect(
    database_url: &str,
) -> Result<(Client, JoinHandle<()>), tokio_postgres::Error> {
    let mut config: Config = database_url.parse()?;
    config.options("-c search_path=pg_catalog,public,pg_temp");
    let (client, connection) = config.connect(NoTls).await?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok((client, driver))
}

/// Apply all repository-owned workflow migrations using a local plaintext connection.
///
/// # Errors
/// Returns a migration error when `PostgreSQL` is unavailable or retained schema state
/// is unsupported.
pub async fn migrate(database_url: &str) -> Result<(), MigrationError> {
    let (mut client, driver) = connect(database_url).await?;
    let result = migrations::migrate(&mut client).await;
    driver.abort();
    result
}

/// Apply migrations and grant one distinct least-privilege runtime role.
///
/// # Errors
/// Returns a migration or runtime-role error when the schema cannot be prepared safely.
pub async fn migrate_for_runtime(
    database_url: &str,
    runtime_role: &str,
) -> Result<(), MigrationError> {
    let (mut client, driver) = connect(database_url).await?;
    let result = migrations::migrate_for_runtime(&mut client, runtime_role).await;
    driver.abort();
    result
}

/// Validate that the configured database has exactly the supported workflow schema.
///
/// # Errors
/// Returns a migration error for unavailable, incomplete, modified, or future state.
pub async fn validate(database_url: &str) -> Result<(), MigrationError> {
    let (client, driver) = connect(database_url).await?;
    let result = migrations::validate(&client).await;
    driver.abort();
    result
}

/// Validate schema, privileges, and the exact connected runtime role.
///
/// # Errors
/// Returns a validation error for unavailable, modified, privileged, or misbound state.
pub async fn validate_for_runtime(
    database_url: &str,
    runtime_role: &str,
) -> Result<(), MigrationError> {
    let (client, driver) = connect(database_url).await?;
    let result = migrations::validate_for_runtime(&client, runtime_role).await;
    driver.abort();
    result
}

/// Feasibility-only generated snapshot/watch protocol retained until Slice 3.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod watch_v1 {
    tonic::include_proto!("ayni.orchestrator.feasibility.v1");
}

pub mod watch;
