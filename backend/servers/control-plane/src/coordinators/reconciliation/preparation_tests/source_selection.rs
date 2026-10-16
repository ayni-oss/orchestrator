//! Source changes select desired infrastructure without mutating frozen operations.
use super::*;

#[tokio::test]
async fn changed_source_selects_desired_destination_while_old_operations_keep_frozen_inputs()
-> TestResult {
    let mut fixture = Fixture::new().await?;
    let mut desired = fixture.runtime.clone();
    desired.image = format!(
        "ghcr.io/new-destination/runtime:ayni-arm64-{}",
        "e".repeat(64)
    );
    let desired_image = desired.image.clone();
    fixture
        .reconciler
        .configuration
        .profiles
        .insert(fixture.profile.clone(), desired);
    let registry = v1::RuntimeRegistrySelection::default();
    let frozen = fixture
        .reconciler
        .preparation_runtime(&fixture.profile, &fixture.cursor, &registry)
        .await?;
    assert_eq!(frozen.image, fixture.runtime.image);
    let mut next = fixture.cursor.clone();
    next.pending = None;
    let fresh = fixture
        .reconciler
        .preparation_runtime(&fixture.profile, &next, &registry)
        .await?;
    assert_eq!(fresh.image, desired_image);
    let previous = fixture.pending()?.profile_revision.clone();
    let selected = fixture
        .reconciler
        .observe_source_selection(
            &fixture.profile,
            &"b".repeat(40),
            &crate::profiles::ayni::environment::LockIdentity {
                fingerprint: "e".repeat(64),
                ayni_version: "0.15.0".into(),
            },
            Some(previous),
            &v1::RuntimeRegistrySelection::default(),
        )
        .await?;
    let (profile, revision) = fixture
        .reconciler
        .state
        .database
        .execute(move |store| {
            let revision = store.runtime_profile_revision(&selected)?;
            Ok((
                store.runtime_profile(&revision.runtime_profile_id)?,
                revision,
            ))
        })
        .await?;
    assert_eq!(profile.state, v1::RuntimeProfileState::Updating as i32);
    let Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) =
        revision.configuration
    else {
        return Err("managed configuration missing".into());
    };
    assert_eq!(
        configuration.oci_repository,
        "ghcr.io/new-destination/runtime"
    );
    assert_eq!(configuration.lock_fingerprint, "e".repeat(64));
    assert!(
        fixture
            .pending()?
            .frozen_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.image.starts_with("ghcr.io/ayni-oss/sandbox:"))
    );
    Ok(())
}
