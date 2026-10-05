use super::Database;
use crate::adapters::sqlite::{CleanStore, CleanStoreError, v1};
use std::{error::Error, sync::mpsc, time::Duration};

fn database() -> Result<Database, CleanStoreError> {
    Database::start(CleanStore::in_memory()?)
}

fn repository_request() -> v1::CreateRepositoryConnectionRequest {
    v1::CreateRepositoryConnectionRequest {
        idempotency_key: "create".into(),
        display_name: "Repository".into(),
        default_branch: "main".into(),
        provider: Some(v1::RepositoryProvider {
            provider: Some(v1::repository_provider::Provider::Github(
                v1::GithubRepository {
                    repository_id: "42".into(),
                    owner: "owner".into(),
                    name: "repo".into(),
                    installation_id: 7,
                    html_url: "https://github.com/owner/repo".into(),
                },
            )),
        }),
    }
}

#[tokio::test]
async fn synchronous_work_does_not_block_the_single_thread_executor() -> Result<(), Box<dyn Error>>
{
    let database = database()?;
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let operation = tokio::spawn(async move {
        database
            .execute(move |_| {
                let _ = started.send(());
                blocked
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| CleanStoreError::StoreUnavailable)?;
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), ready).await??;
    // This task must run while the database thread is waiting for it.
    tokio::task::yield_now().await;
    release.send(())?;
    operation.await??;
    Ok(())
}

#[tokio::test]
async fn accepted_work_finishes_after_the_caller_disconnects() -> Result<(), Box<dyn Error>> {
    let database = database()?;
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let worker = database.clone();
    let caller = tokio::spawn(async move {
        worker
            .execute(move |store| {
                let _ = started.send(());
                blocked
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| CleanStoreError::StoreUnavailable)?;
                store.create_repository_connection("create", &repository_request())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), ready).await??;
    caller.abort();
    release.send(())?;
    let retained = database
        .execute(|store| store.list_repository_connections(None, None))
        .await?;
    assert_eq!(retained.items.len(), 1);
    let replay = database
        .execute(|store| store.create_repository_connection("create", &repository_request()))
        .await?;
    assert_eq!(retained.items[0].id, replay.id);
    Ok(())
}

#[tokio::test]
async fn notifications_follow_committed_events_and_ignore_rollback() -> Result<(), Box<dyn Error>> {
    let database = database()?;
    let notifications = database.notifications();
    let notified = notifications.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    database
        .execute(|store| store.create_repository_connection("create", &repository_request()))
        .await?;
    tokio::time::timeout(Duration::from_secs(1), notified).await?;

    let notified = notifications.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    database
        .execute(|store| {
            let transaction = store
                .connection
                .transaction()
                .map_err(CleanStoreError::Database)?;
            transaction
                .execute("DELETE FROM resource_events", [])
                .map_err(CleanStoreError::Database)?;
            transaction.rollback().map_err(CleanStoreError::Database)
        })
        .await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), notified)
            .await
            .is_err()
    );
    Ok(())
}
