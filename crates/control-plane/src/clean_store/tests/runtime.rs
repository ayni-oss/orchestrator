use super::*;
use ayni_orchestrator_runtime::{
    managed_runtime::VerifiedRuntime, runtime_execution::VerificationReport,
};

fn report(revision: &str) -> VerificationReport {
    VerificationReport {
        revision: revision.into(),
        lock_digest: DIGEST.into(),
        check_exit_code: 1,
        runtime: VerifiedRuntime {
            lock_fingerprint: format!("sha256:{}", "c".repeat(64)),
            platform: "linux/amd64".into(),
            ayni_version: ayni_orchestrator_runtime::ayni_release::CLI_VERSION.into(),
            certificate_key_id: "test-producer".into(),
            metadata_digest: DIGEST.into(),
            protected_content_root: DIGEST.into(),
        },
    }
}

#[test]
fn admission_requires_the_verified_source_revision() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    store.observe_repository_snapshot(&repository.id, SECOND_REVISION)?;
    let mut request = task_request(&repository.id, &profile.id, "new");
    request
        .request
        .as_mut()
        .ok_or(CleanStoreError::CorruptState)?
        .repository_revision = SECOND_REVISION.into();
    assert!(matches!(
        store.create_development_task("new", &request),
        Err(CleanStoreError::ProfileNotReady)
    ));
    Ok(())
}

#[test]
fn fresh_source_verification_reuses_digest_but_not_stale_revision() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (_, attempt) = store.create_development_task(
        "initial",
        &task_request(&repository.id, &profile.id, "initial"),
    )?;
    let revision = store.prepare_runtime_revision(
        &profile.id,
        SECOND_REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        DIGEST,
    )?;
    assert_eq!(
        store.runtime_profile(&profile.id)?.state,
        v1::RuntimeProfileState::Updating as i32
    );
    assert!(
        store
            .activate_runtime(
                &profile.id,
                &revision,
                "registry.test/team/runtime:stable",
                DIGEST,
                &report(REVISION)
            )
            .is_err()
    );
    store.activate_runtime(
        &profile.id,
        &revision,
        "registry.test/team/runtime:stable",
        DIGEST,
        &report(SECOND_REVISION),
    )?;
    store.activate_runtime(
        &profile.id,
        &revision,
        "registry.test/team/runtime:stable",
        DIGEST,
        &report(SECOND_REVISION),
    )?;
    assert_eq!(
        store.runtime_profile(&profile.id)?.state,
        v1::RuntimeProfileState::Ready as i32
    );
    assert!(
        store
            .activate_runtime(
                &profile.id,
                &revision,
                "registry.test/team/runtime:stable",
                SECOND_DIGEST,
                &report(SECOND_REVISION)
            )
            .is_err()
    );
    assert_eq!(
        store.development_attempt(&attempt.id)?.snapshot,
        attempt.snapshot
    );
    Ok(())
}

#[test]
fn source_policy_change_and_revert_create_new_selected_revisions() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let first = store.prepare_runtime_revision(
        &profile.id,
        REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        DIGEST,
    )?;
    assert_eq!(
        first,
        store.prepare_runtime_revision(
            &profile.id,
            REVISION,
            &"c".repeat(64),
            "registry.test/team/runtime",
            DIGEST
        )?
    );
    let second = store.prepare_runtime_revision(
        &profile.id,
        REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        SECOND_DIGEST,
    )?;
    let reverted = store.prepare_runtime_revision(
        &profile.id,
        REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        DIGEST,
    )?;
    assert_ne!(first, second);
    assert_ne!(first, reverted);
    assert_eq!(
        store
            .runtime_profile(&profile.id)?
            .active_revision_id
            .as_deref(),
        Some(reverted.as_str())
    );
    Ok(())
}

#[test]
fn runtime_cursor_and_status_survive_restart() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("runtime.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let cursor = store.runtime_cursor(&profile.id)?;
    store.save_runtime_cursor(&profile.id, &cursor)?;
    let metadata = store.runtime_profile(&profile.id)?.reconciliation_metadata;
    assert!(metadata.iter().any(|m| m.key == "next-retry"));
    drop(store);
    let store = CleanStore::open(path, 3)?;
    assert_eq!(store.runtime_cursor(&profile.id)?.metadata(), metadata);
    Ok(())
}
