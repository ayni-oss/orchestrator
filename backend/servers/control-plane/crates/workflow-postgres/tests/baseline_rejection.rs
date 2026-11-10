//! A fresh-install baseline must reject retained pre-baseline databases without mutation.
use ayni_orchestrator_workflow_postgres::{CURRENT_SCHEMA_VERSION, MigrationError, migrate_client};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use tokio_postgres::{Config, NoTls};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL acceptance fixture"]
async fn retained_pre_baseline_history_is_rejected_without_changes() -> TestResult {
    let config: Config = std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?.parse()?;
    let (admin, connection) = config.connect(NoTls).await?;
    let admin_driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    let database = format!("baseline_rejection_{}", Uuid::now_v7().simple());
    admin
        .batch_execute(&format!("CREATE DATABASE {database}"))
        .await?;
    let mut isolated = config.clone();
    isolated.dbname(&database);
    let (mut client, connection) = isolated.connect(NoTls).await?;
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    client.batch_execute("CREATE TABLE workflow_schema_migrations (version bigint PRIMARY KEY CHECK(version>0), description text NOT NULL, checksum text NOT NULL, applied_at timestamptz NOT NULL DEFAULT clock_timestamp()); CREATE TABLE retained_evidence (value text NOT NULL); INSERT INTO retained_evidence VALUES ('must survive');").await?;
    for prior in [1_i64, 2, 41, 42] {
        client.execute("INSERT INTO workflow_schema_migrations(version,description,checksum) VALUES ($1,'prior baseline','retained-checksum')", &[&prior]).await?;
        assert!(matches!(
            migrate_client(&mut client).await,
            Err(MigrationError::FreshDatabaseRequired)
        ));
        let row = client.query_one("SELECT value, (SELECT max(version) FROM workflow_schema_migrations), to_regclass('public.workflow_projects') IS NULL FROM retained_evidence", &[]).await?;
        assert_eq!(row.get::<_, String>(0), "must survive");
        assert_eq!(row.get::<_, i64>(1), prior);
        assert!(row.get::<_, bool>(2));
        client
            .batch_execute("DELETE FROM workflow_schema_migrations")
            .await?;
    }
    // A valid prefix of the new catalog can resume, but a changed applied file
    // must be rejected before installing any later layers.
    let hierarchy = include_str!("../migrations/001_hierarchy.sql");
    client.batch_execute(hierarchy).await?;
    client.batch_execute("INSERT INTO workflow_schema_migrations(version,description,checksum) VALUES (1,'Hierarchy','corrupt-checksum')").await?;
    assert!(matches!(
        migrate_client(&mut client).await,
        Err(MigrationError::ChecksumMismatch)
    ));
    let row = client.query_one("SELECT value, (SELECT count(*) FROM workflow_schema_migrations), to_regclass('public.workflow_connections') IS NULL FROM retained_evidence", &[]).await?;
    assert_eq!(row.get::<_, String>(0), "must survive");
    assert_eq!(row.get::<_, i64>(1), 1);
    assert!(row.get::<_, bool>(2));
    let checksum = format!("sha256:{:x}", Sha256::digest(hierarchy.as_bytes()));
    client
        .execute(
            "UPDATE workflow_schema_migrations SET checksum=$1 WHERE version=1",
            &[&checksum],
        )
        .await?;
    client.batch_execute("DROP TABLE retained_evidence").await?;
    migrate_client(&mut client).await?;
    migrate_client(&mut client).await?;
    let count: i64 = client
        .query_one("SELECT count(*) FROM workflow_schema_migrations", &[])
        .await?
        .get(0);
    assert_eq!(count, CURRENT_SCHEMA_VERSION);
    drop(client);
    driver.abort();
    admin
        .batch_execute(&format!("DROP DATABASE {database}"))
        .await?;
    admin_driver.abort();
    Ok(())
}
