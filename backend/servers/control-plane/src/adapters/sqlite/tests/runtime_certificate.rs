use super::*;
fn certificate(id: &str, default: bool) -> v1::Certificate {
    v1::Certificate {
        id: id.into(),
        revision: "1".into(),
        key_id: "release".into(),
        public_key: "a".repeat(64),
        secret_name: format!("signing-{id}-1"),
        enabled: true,
        status: "ready".into(),
        is_default: default,
        resource_version: 1,
        ..Default::default()
    }
}
fn selection(
    profile: &v1::RuntimeProfile,
    id: &str,
    key: &str,
) -> v1::SetRuntimeCertificateRequest {
    v1::SetRuntimeCertificateRequest {
        idempotency_key: key.into(),
        runtime_profile_id: profile.id.clone(),
        expected_resource_version: profile.resource_version,
        certificate_id: id.into(),
    }
}
#[test]
fn selection_retires_readiness_and_preserves_frozen_tasks() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (frozen, _) = store.create_development_task(
        "frozen",
        &task_request(&store, &repository.id, &profile.id, "frozen")?,
    )?;
    let certificate = certificate("certificate-first", true);
    store.save_certificate(&certificate)?;
    let request = selection(&profile, &certificate.id, "select");
    let selected = store.set_runtime_certificate("select", &request)?;
    assert_ne!(selected.active_revision_id, profile.active_revision_id);
    assert!(selected.active_runtime_artifact_id.is_none());
    assert_eq!(store.development_task(&frozen.id)?, frozen);
    assert_eq!(store.set_runtime_certificate("select", &request)?, selected);
    assert!(matches!(
        store.set_runtime_certificate("stale", &request),
        Err(CleanStoreError::StaleVersion)
    ));
    let mut conflicting = request;
    conflicting.certificate_id.clear();
    assert!(matches!(
        store.set_runtime_certificate("select", &conflicting),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    assert_eq!(store.certificate_runtime_usage(&certificate.id)?, 1);
    Ok(())
}
#[test]
fn same_key_id_rotation_and_default_switch_retire_selected_revision() -> Result<(), CleanStoreError>
{
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let mut first = certificate("certificate-first", true);
    store.save_certificate(&first)?;
    store.invalidate_certificate_runtime_profiles()?;
    let initial = store.runtime_profile(&profile.id)?;
    first.public_key = "b".repeat(64);
    first.revision = "2".into();
    first.secret_name = "signing-first-2".into();
    store.save_certificate(&first)?;
    store.invalidate_certificate_runtime_profiles()?;
    let rotated = store.runtime_profile(&profile.id)?;
    assert_ne!(initial.active_revision_id, rotated.active_revision_id);
    assert!(rotated.active_runtime_artifact_id.is_none());
    store.invalidate_certificate_runtime_profiles()?;
    assert_eq!(store.runtime_profile(&profile.id)?, rotated);
    first.is_default = false;
    store.save_certificate(&first)?;
    let second = certificate("certificate-second", true);
    store.save_certificate(&second)?;
    store.invalidate_certificate_runtime_profiles()?;
    let switched = store.runtime_profile(&profile.id)?;
    assert_ne!(switched.active_revision_id, rotated.active_revision_id);
    assert_eq!(
        switched.effective_certificate.map(|c| c.id),
        Some(second.id)
    );
    Ok(())
}
#[test]
fn explicit_certificate_stays_fixed_when_default_changes() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let mut first = certificate("certificate-first", true);
    store.save_certificate(&first)?;
    let selected =
        store.set_runtime_certificate("select", &selection(&profile, &first.id, "select"))?;
    first.is_default = false;
    store.save_certificate(&first)?;
    store.save_certificate(&certificate("certificate-second", true))?;
    store.invalidate_certificate_runtime_profiles()?;
    assert_eq!(store.runtime_profile(&profile.id)?, selected);
    Ok(())
}
#[test]
fn pending_frozen_build_keeps_retired_certificate_referenced() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let first = certificate("certificate-first", true);
    store.save_certificate(&first)?;
    let selected =
        store.set_runtime_certificate("select", &selection(&profile, &first.id, "select"))?;
    let cursor = serde_json::json!({"sequence":1,"next_at":0,"last_success":null,"failures":0,"bindings":{},"status":{},"pending":{"name":"old","profile_revision":selected.active_revision_id,"request":{"repository_url":"https://github.com/owner/repo.git","revision":REVISION,"expected":{"lock_fingerprint":DIGEST,"platform":"linux/amd64","ayni_version":"0.14.5"}},"image":"ghcr.io/owner/repo:lock","started_at":1,"source_digest":DIGEST,"lock_digest":DIGEST,"build_name":"build-old","build_observed":false,"build_started":true,"force_rebuild":false,"certificate":{"id":first.id,"revision":"1","key_id":"release","public_key":"a".repeat(64),"secret_name":first.secret_name,"enabled":true,"status":"ready"}}});
    store.connection.execute("INSERT INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES(?1,?2,1,1)",params![format!("managed:{}",profile.id),cursor.to_string()]).map_err(CleanStoreError::Database)?;
    let second = certificate("certificate-second", false);
    store.save_certificate(&second)?;
    store.set_runtime_certificate("switch", &selection(&selected, &second.id, "switch"))?;
    assert_eq!(store.certificate_runtime_usage(&first.id)?, 1);
    Ok(())
}

#[test]
fn disabled_certificates_and_custom_profiles_reject_assignment() -> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (_, profile, _) = ready_profile(&mut store)?;
    let mut disabled = certificate("certificate-disabled", false);
    disabled.enabled = false;
    store.save_certificate(&disabled)?;
    assert!(matches!(
        store.set_runtime_certificate("disabled", &selection(&profile, &disabled.id, "disabled")),
        Err(CleanStoreError::InvalidTransition)
    ));
    let mut custom = profile;
    custom.kind = v1::RuntimeProfileKind::CustomRuntime as i32;
    store
        .connection
        .execute(
            "UPDATE runtime_profiles SET kind=?1,payload=?2 WHERE id=?3",
            params![custom.kind, custom.encode_to_vec(), custom.id],
        )
        .map_err(CleanStoreError::Database)?;
    assert!(matches!(
        store.set_runtime_certificate("custom", &selection(&custom, "", "custom")),
        Err(CleanStoreError::InvalidTransition)
    ));
    assert!(store.effective_runtime_certificate(&custom)?.is_none());
    Ok(())
}

#[test]
fn old_certificate_observation_cannot_activate_after_selection_changes()
-> Result<(), CleanStoreError> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (_, attempt) = store.create_development_task(
        "frozen-rotation",
        &task_request(&store, &repository.id, &profile.id, "frozen-rotation")?,
    )?;
    let old_revision = profile
        .active_revision_id
        .as_deref()
        .ok_or(CleanStoreError::CorruptState)?;
    let first = certificate("certificate-first", true);
    store.save_certificate(&first)?;
    let selected =
        store.set_runtime_certificate("select", &selection(&profile, &first.id, "select"))?;
    store.record_runtime_artifact(&profile.id, verified_artifact(old_revision, SECOND_DIGEST))?;
    let current = store.runtime_profile(&profile.id)?;
    assert_eq!(current.active_revision_id, selected.active_revision_id);
    assert!(current.active_runtime_artifact_id.is_none());
    assert_eq!(
        store.development_attempt(&attempt.id)?.snapshot,
        attempt.snapshot
    );
    Ok(())
}
