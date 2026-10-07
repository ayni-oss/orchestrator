use super::*;

fn registry(id: &str, namespace: &str) -> v1::Connection {
    v1::Connection {
        id: id.into(),
        kind: "ghcr".into(),
        management_source: "ui".into(),
        image_namespace: namespace.into(),
        status: "ready".into(),
        enabled: true,
        secret_name: format!("{id}-push"),
        pull_secret_name: format!("{id}-pull"),
        resource_version: 1,
        ..Default::default()
    }
}

fn assign(
    profile: &v1::RuntimeProfile,
    registry: &v1::Connection,
    key: &str,
) -> v1::SetRuntimeRegistryRequest {
    v1::SetRuntimeRegistryRequest {
        idempotency_key: key.into(),
        runtime_profile_id: profile.id.clone(),
        expected_resource_version: profile.resource_version,
        registry_connection_id: registry.id.clone(),
        image_namespace: registry.image_namespace.clone(),
    }
}

#[test]
fn registry_updates_are_isolated_idempotent_and_invalidate_admission() -> Result<(), CleanStoreError>
{
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let snapshot = store.observe_repository_snapshot(&repository.id, REVISION)?;
    let mut second_request = profile_request("second", &repository.id, &snapshot.id);
    if let Some(v1::runtime_profile_configuration::Configuration::AyniManaged(configuration)) =
        second_request
            .configuration
            .as_mut()
            .and_then(|c| c.configuration.as_mut())
    {
        configuration.lock_fingerprint = "e".repeat(64);
    }
    let (second, _) = store.create_runtime_profile("second", &second_request)?;
    let registry = registry("connection-first", "first");
    store.save_connection(&registry)?;
    let request = assign(&profile, &registry, "assign-first");
    let updated = store.set_runtime_registry("assign-first", &request)?;
    assert_eq!(
        updated
            .registry
            .as_ref()
            .map(|v| v.registry_connection_id.as_str()),
        Some(registry.id.as_str())
    );
    assert_eq!(store.runtime_profile(&second.id)?, second);
    assert_eq!(store.repository_connection(&repository.id)?, repository);
    assert!(updated.active_runtime_artifact_id.is_none());
    assert_ne!(updated.active_revision_id, profile.active_revision_id);
    assert_eq!(updated.state, v1::RuntimeProfileState::Updating as i32);
    assert_eq!(
        store.set_runtime_registry("assign-first", &request)?,
        updated
    );
    assert!(matches!(
        store.set_runtime_registry("stale", &request),
        Err(CleanStoreError::StaleVersion)
    ));
    let mut conflicting = request;
    conflicting.image_namespace = "elsewhere".into();
    assert!(matches!(
        store.set_runtime_registry("assign-first", &conflicting),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    assert!(matches!(
        store.create_development_task(
            "blocked",
            &task_request(&store, &repository.id, &profile.id, "blocked")?
        ),
        Err(CleanStoreError::ProfileNotReady)
    ));
    assert!(store.registry_connection_in_use(&registry.id)?);
    Ok(())
}

#[test]
fn explicit_selection_replaces_legacy_defaults_and_survives_restart() -> Result<(), CleanStoreError>
{
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("state.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let old = registry("connection-old", "old");
    let new = registry("connection-new", "new");
    store.save_connection(&old)?;
    store.save_connection(&new)?;
    store.set_repository_registry(
        "default",
        &v1::SetRepositoryRegistryRequest {
            idempotency_key: "default".into(),
            repository_connection_id: repository.id.clone(),
            expected_resource_version: repository.resource_version,
            registry_connection_id: old.id.clone(),
            image_namespace: old.image_namespace.clone(),
        },
    )?;
    // Existing schema-six protobuf payloads can lack the additive registry field.
    let mut legacy = profile;
    legacy.registry = None;
    store
        .connection
        .execute(
            "UPDATE runtime_profiles SET payload=?1 WHERE id=?2",
            params![legacy.encode_to_vec(), legacy.id],
        )
        .map_err(CleanStoreError::Database)?;
    assert_eq!(
        store.runtime_registry(&legacy)?.registry_connection_id,
        old.id
    );
    assert!(store.registry_connection_in_use(&old.id)?);
    let updated = store.set_runtime_registry("move", &assign(&legacy, &new, "move"))?;
    assert!(!store.registry_connection_in_use(&old.id)?);
    assert!(store.registry_connection_in_use(&new.id)?);
    drop(store);
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(store.runtime_profile(&legacy.id)?, updated);
    assert_eq!(
        store.runtime_registry(&updated)?.registry_connection_id,
        new.id
    );
    Ok(())
}

#[test]
fn changed_destinations_keep_frozen_attempt_credentials_and_reject_old_activation()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let first = registry("connection-first", "first");
    let second = registry("connection-second", "second");
    store.save_connection(&first)?;
    store.save_connection(&second)?;
    let selected = store.set_runtime_registry("first", &assign(&profile, &first, "first"))?;
    let revision_id = selected
        .active_revision_id
        .as_deref()
        .ok_or(CleanStoreError::CorruptState)?;
    let mut artifact = verified_artifact(revision_id, DIGEST);
    artifact.oci_repository = "ghcr.io/first/ayni-oss/sandbox-ayni".into();
    store.record_runtime_artifact(&selected.id, artifact)?;
    let ready = store.runtime_profile(&selected.id)?;
    let (_, attempt) = store.create_development_task(
        "frozen",
        &task_request(&store, &repository.id, &ready.id, "frozen")?,
    )?;
    let frozen = attempt
        .snapshot
        .as_ref()
        .and_then(|s| s.runtime.as_ref())
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(frozen.registry_connection_id, first.id);
    assert_eq!(frozen.pull_secret_name, first.pull_secret_name);
    let changed = store.set_runtime_registry("second", &assign(&ready, &second, "second"))?;
    assert!(store.registry_connection_in_use(&first.id)?);
    assert!(store.connection_secret_in_use(&first.pull_secret_name)?);
    // An old verification can be retained as historical evidence but cannot gain
    // admission authority after the destination changes.
    store.record_runtime_artifact(&ready.id, verified_artifact(revision_id, SECOND_DIGEST))?;
    assert_eq!(
        store.runtime_profile(&ready.id)?.active_revision_id,
        changed.active_revision_id
    );
    assert!(
        store
            .runtime_profile(&ready.id)?
            .active_runtime_artifact_id
            .is_none()
    );
    let stored = store.development_attempt(&attempt.id)?;
    assert_eq!(stored.snapshot, attempt.snapshot);
    Ok(())
}

#[test]
fn stale_source_observation_cannot_overwrite_a_new_runtime_destination()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let observed_registry = store.runtime_registry(&profile)?;
    let initial_cursor_metadata = store.runtime_cursor(&profile.id)?.metadata();
    let registry = registry("connection-new", "new");
    store.save_connection(&registry)?;
    let updated = store.set_runtime_registry("new", &assign(&profile, &registry, "new"))?;
    let command = crate::profiles::ayni::runtime_selection::command(
        &profile.id,
        REVISION,
        &"c".repeat(64),
        "ghcr.io/obsolete/ayni-oss/sandbox-ayni",
        DIGEST,
    )
    .map_err(|_| CleanStoreError::CorruptState)?;
    assert!(matches!(
        store.select_observed_runtime_revision(
            &command,
            profile.active_revision_id.as_deref(),
            &observed_registry
        ),
        Err(CleanStoreError::StaleVersion)
    ));
    assert_eq!(store.runtime_profile(&profile.id)?, updated);
    assert_eq!(
        store.runtime_cursor(&profile.id)?.metadata(),
        initial_cursor_metadata
    );
    Ok(())
}
