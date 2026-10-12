use super::*;
use crate::profiles::ayni::preparation_state::RuntimeCursorExt;
use ayni_orchestrator_profile_ayni::{
    managed_runtime::VerifiedRuntime, runtime_execution::VerificationReport,
};
use ayni_orchestrator_runtime::coding::{QualityOutcome, QualityResult};

pub(super) fn report(revision: &str) -> VerificationReport {
    VerificationReport {
        revision: revision.into(),
        lock_digest: DIGEST.into(),
        check_exit_code: 1,
        quality: QualityResult {
            outcome: QualityOutcome::Failed,
            evidence: None,
            completed_targets: None,
            skipped_targets: None,
            warning_offenders: None,
            failing_offenders: None,
            rows: vec![],
            findings: vec!["Full check failed".into()],
        },
        runtime: VerifiedRuntime {
            lock_fingerprint: format!("sha256:{}", "c".repeat(64)),
            platform: "linux/amd64".into(),
            ayni_version: "0.15.0".into(),
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
    assert!(metadata.iter().any(|m| m.key == "retry-eligibility"));
    drop(store);
    let store = CleanStore::open(path, 3)?;
    assert_eq!(store.runtime_cursor(&profile.id)?.metadata(), metadata);
    Ok(())
}

#[test]
fn runtime_reads_recover_verified_trust_from_legacy_cursor_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("runtime.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let mut cursor = store.runtime_cursor(&profile.id)?;
    cursor
        .status
        .insert("certificate-trust-state".into(), "pending".into());
    store.save_runtime_cursor(&profile.id, &cursor)?;
    cursor.trust = Some(crate::profiles::ayni::preparation_state::TrustUpdate {
        generation: "generation".into(),
        head: REVISION.into(),
        job: "trust-job".into(),
        attempt: 0,
        result: None,
        pull_number: None,
        state: "verified".into(),
        merge_revision: None,
    });
    // Reproduce the old interrupted save: verified durable evidence, pending projection.
    store.connection.execute(
        "UPDATE reconciliation_cursors SET value=?1 WHERE name=?2",
        params![
            serde_json::to_string(&cursor)?,
            format!("managed:{}", profile.id)
        ],
    )?;
    drop(store);
    let store = CleanStore::open(path, 3)?;
    let described = store.describe_runtime_profile(store.runtime_profile(&profile.id)?)?;
    assert!(
        described
            .reconciliation_metadata
            .iter()
            .any(|entry| entry.key == "certificate-trust-state" && entry.value == "verified")
    );
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

pub(super) fn select(
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
        "0.15.0",
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

#[test]
fn unsupported_historical_contract_blocks_new_admission_and_preserves_frozen_tasks()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (task, _) = store.create_development_task(
        "retained-task",
        &task_request(&store, &repository.id, &profile.id, "retained-task")?,
    )?;
    let mut revision = store.runtime_profile_revision(
        profile
            .active_revision_id
            .as_deref()
            .ok_or(CleanStoreError::CorruptState)?,
    )?;
    let Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) =
        &mut revision.configuration
    else {
        return Err(CleanStoreError::CorruptState);
    };
    configuration.ayni_version = "0.14.5".into();
    store
        .connection
        .execute(
            "UPDATE runtime_profile_revisions SET payload=?1 WHERE id=?2",
            params![revision.encode_to_vec(), revision.id],
        )
        .map_err(CleanStoreError::Database)?;
    assert!(matches!(
        store.create_development_task(
            "new-task",
            &task_request(&store, &repository.id, &profile.id, "new-task")?
        ),
        Err(CleanStoreError::ProfileNotReady)
    ));
    assert_eq!(store.development_task(&task.id)?, task);
    Ok(())
}
