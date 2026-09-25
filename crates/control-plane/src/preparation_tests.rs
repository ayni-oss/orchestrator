use super::*;
use crate::development::RepositoryBinding;

fn binding() -> RepositoryBinding {
    RepositoryBinding {
        id: "sandbox".into(),
        owner: "ayni-oss".into(),
        repository: "sandbox".into(),
        installation_id: 1,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
    }
}

#[test]
fn preparation_is_content_addressed_and_reused_only_after_success() -> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&binding())?;
    let input = PrepareRepositoryInput {
        revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
    };
    let first_key = IdempotencyKey::try_from("prepare-cold".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let cold = store.prepare_repository("sandbox", &first_key, &input)?;
    assert_eq!(cold.status, PreparationStatus::Queued);
    assert!(!cold.reused);
    assert!(store.set_preparation_status(
        &cold.id,
        PreparationStatus::Queued,
        PreparationStatus::Preparing,
    )?);
    assert!(store.complete_preparation(&cold.id, 47)?);

    let warm_key = IdempotencyKey::try_from("prepare-warm".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let warm = store.prepare_repository("sandbox", &warm_key, &input)?;
    assert_eq!(warm.id, cold.id);
    assert_eq!(warm.status, PreparationStatus::Ready);
    assert!(warm.reused);
    assert_eq!(warm.reuse_count, 1);
    assert_eq!(warm.duration_seconds, Some(47));
    Ok(())
}

#[test]
fn changed_revision_never_uses_an_older_ready_preparation() -> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&binding())?;
    let old_revision = "8a555ada2f6e3ee41de2485ed13da508beaebaeb";
    let key = IdempotencyKey::try_from("prepare-old".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let old = store.prepare_repository(
        "sandbox",
        &key,
        &PrepareRepositoryInput {
            revision: old_revision.into(),
        },
    )?;
    assert!(store.set_preparation_status(
        &old.id,
        PreparationStatus::Queued,
        PreparationStatus::Preparing,
    )?);
    assert!(store.complete_preparation(&old.id, 1)?);
    assert!(
        store
            .ready_preparation(
                "sandbox",
                "4e67795c0cba21bd82c21bdd6c921417fbe29ff6",
                &binding().lock_sha256,
            )
            .is_err()
    );
    Ok(())
}
