use super::*;
use ayni_orchestrator_profile_ayni::{
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
            ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION.into(),
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
    let mut request = task_request(&store, &repository.id, &profile.id, "new")?;
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
        &task_request(&store, &repository.id, &profile.id, "initial")?,
    )?;
    let revision = select(
        &mut store,
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
        activate(
            &mut store,
            &profile.id,
            &revision,
            "registry.test/team/runtime:stable",
            DIGEST,
            &report(REVISION)
        )
        .is_err()
    );
    activate(
        &mut store,
        &profile.id,
        &revision,
        "registry.test/team/runtime:stable",
        DIGEST,
        &report(SECOND_REVISION),
    )?;
    activate(
        &mut store,
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
        activate(
            &mut store,
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
    let first = select(
        &mut store,
        &profile.id,
        REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        DIGEST,
    )?;
    assert_eq!(
        first,
        select(
            &mut store,
            &profile.id,
            REVISION,
            &"c".repeat(64),
            "registry.test/team/runtime",
            DIGEST
        )?
    );
    let second = select(
        &mut store,
        &profile.id,
        REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        SECOND_DIGEST,
    )?;
    let reverted = select(
        &mut store,
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

fn activate(
    store: &mut CleanStore,
    profile: &str,
    revision: &str,
    image: &str,
    digest: &str,
    report: &VerificationReport,
) -> Result<(), CleanStoreError> {
    let command = crate::profiles::ayni::runtime_activation::command(
        profile, revision, image, digest, report,
    )
    .map_err(|_| CleanStoreError::CorruptState)?;
    ayni_orchestrator_application::runtime_activation::activate(store, &command)
        .map_err(super::super::runtime_activation::activation_error)
}

fn select(
    store: &mut CleanStore,
    profile: &str,
    revision: &str,
    fingerprint: &str,
    repository: &str,
    source_digest: &str,
) -> Result<String, CleanStoreError> {
    let command = crate::profiles::ayni::runtime_selection::command(
        profile,
        revision,
        fingerprint,
        repository,
        source_digest,
    )
    .map_err(|_| CleanStoreError::CorruptState)?;
    ayni_orchestrator_application::runtime_selection::select(store, &command)
        .map_err(super::super::runtime_selection::selection_error)
}

#[test]
fn runtime_activation_stale_commit_preserves_artifacts_and_events()
-> Result<(), Box<dyn std::error::Error>> {
    use ayni_orchestrator_application::runtime_activation::{Mutation, Repository};
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let revision = select(
        &mut store,
        &profile.id,
        SECOND_REVISION,
        &"c".repeat(64),
        "registry.test/team/runtime",
        DIGEST,
    )?;
    let command = crate::profiles::ayni::runtime_activation::command(
        &profile.id,
        &revision,
        "registry.test/team/runtime:stable",
        DIGEST,
        &report(SECOND_REVISION),
    )?;
    let facts = Repository::profile(&store, &profile.id)?;
    store.runtime_failure(&profile.id, "test_failure")?;
    let before: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert!(matches!(
        Repository::commit(&mut store, &command, &facts, Mutation::RecordArtifact),
        Err(CleanStoreError::StaleVersion)
    ));
    let after: i64 =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM resource_events", [], |row| row.get(0))?;
    assert_eq!(before, after);
    assert!(
        store
            .runtime_profile(&profile.id)?
            .active_runtime_artifact_id
            .is_none()
    );
    assert!(
        store
            .list_runtime_artifacts(&profile.id, Some(&revision), None, None)?
            .items
            .is_empty()
    );
    Ok(())
}
