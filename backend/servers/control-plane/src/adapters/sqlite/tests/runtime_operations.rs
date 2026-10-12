//! Runtime progression commits receipt, cursor, projection and outbox together.
use super::verification::prepared;
use super::*;
use crate::profiles::ayni::preparation_state::{RuntimeCursor, TrustUpdate};
use ayni_orchestrator_application::runtime_preparation::ImagePreparationPhase;
use ayni_orchestrator_events::{
    EventIdentity, operations,
    v1::{
        RuntimeOperationBinding, RuntimeOperationEvidenceReference, RuntimeOperationKind,
        RuntimeOperationResultAvailable,
    },
};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) fn seed_publication(
    store: &mut CleanStore,
    profile: &str,
    cursor: &mut RuntimeCursor,
    reporting: &ayni_orchestrator_runtime::verification::ReportingConfiguration,
) -> TestResult {
    let execution = "runtime-decision-smoke";
    cursor.builder.job = Some(execution.into());
    cursor
        .preparation
        .as_mut()
        .ok_or("preparation")?
        .operation_reporting = Some(reporting.clone());
    store.save_runtime_cursor(profile, cursor)?;
    let p = cursor.preparation.as_ref().ok_or("preparation")?;
    let runtime = store.runtime_profile(profile)?;
    let identity = EventIdentity {
        source: reporting.source.clone(),
        id: Uuid::now_v7().to_string(),
    };
    let payload = RuntimeOperationResultAvailable {
        binding: Some(RuntimeOperationBinding {
            repository_connection_id: runtime.repository_connection_id,
            runtime_profile_id: profile.into(),
            runtime_profile_revision_id: runtime.active_revision_id.ok_or("revision")?,
            preparation_id: p.generation.clone(),
            execution_id: execution.into(),
            pod_uid: "synthetic-pod".into(),
            request_digest: DIGEST.into(),
            source_revision: p.source_revision.clone(),
            lock_digest: p.lock_digest.clone(),
            policy_digest: p.policy_digest.clone(),
            platform: p.platform.clone(),
        }),
        kind: RuntimeOperationKind::Discovery as i32,
        evidence: Some(RuntimeOperationEvidenceReference {
            evidence_id: "runtime-operation-json".into(),
            sha256: DIGEST.into(),
            size_bytes: 100,
            contract: operations::CONTRACT.into(),
        }),
    };
    store.receive_runtime_operation(&identity, &payload)?;
    cursor
        .preparation
        .as_mut()
        .ok_or("preparation")?
        .operation_observations
        .insert(execution.into(), identity);
    store.save_runtime_operation_cursor(profile, cursor)?;
    cursor.builder.job = None;
    store.save_runtime_cursor(profile, cursor)?;
    Ok(())
}

fn setup(
    store: &mut CleanStore,
    kind: RuntimeOperationKind,
) -> TestResult<(
    String,
    RuntimeCursor,
    RuntimeCursor,
    EventIdentity,
    RuntimeOperationResultAvailable,
)> {
    let (profile, mut cursor, _, _) = prepared(store)?;
    let source = "ayni://orchestrator/supervisors/test";
    let preparation = cursor.preparation.as_mut().ok_or("preparation")?;
    preparation.operation_reporting = Some(
        ayni_orchestrator_runtime::verification::ReportingConfiguration {
            source: source.into(),
            endpoint: "http://control-plane/events/verification".into(),
        },
    );
    let p = cursor.pending.as_mut().ok_or("pending")?;
    let revision = p.profile_revision.clone();
    let execution = match kind {
        RuntimeOperationKind::Discovery => {
            p.phase = ImagePreparationPhase::DiscoveringStarted;
            ayni_orchestrator_kubernetes::execution::operation_name(
                &format!("discover:{}", p.name),
                0,
            )
        }
        RuntimeOperationKind::Trust => {
            cursor.trust = Some(TrustUpdate {
                generation: "trust-generation".into(),
                head: SECOND_REVISION.into(),
                job: "trust-one".into(),
                attempt: 0,
                result: None,
                pull_number: None,
                state: "pending".into(),
                merge_revision: None,
            });
            "trust-one".into()
        }
        RuntimeOperationKind::Build => {
            p.build_name = Some("build-one".into());
            p.phase = ImagePreparationPhase::BuildingStarted;
            "build-one".into()
        }
        RuntimeOperationKind::Publication => {
            p.build_name = Some("build-one".into());
            p.candidate_image_id = Some(DIGEST.into());
            p.publish_name = Some("publish-one".into());
            p.phase = ImagePreparationPhase::Publishing;
            "publish-one".into()
        }
        RuntimeOperationKind::Unspecified => return Err("kind".into()),
    };
    store.save_runtime_cursor(&profile, &cursor)?;
    let preparation = cursor.preparation.as_ref().ok_or("preparation")?;
    let identity = EventIdentity {
        source: source.into(),
        id: Uuid::now_v7().to_string(),
    };
    let payload = RuntimeOperationResultAvailable {
        binding: Some(RuntimeOperationBinding {
            repository_connection_id: store.runtime_profile(&profile)?.repository_connection_id,
            runtime_profile_id: profile.clone(),
            runtime_profile_revision_id: revision,
            preparation_id: preparation.generation.clone(),
            execution_id: execution.clone(),
            pod_uid: "pod-one".into(),
            request_digest: DIGEST.into(),
            source_revision: preparation.source_revision.clone(),
            lock_digest: preparation.lock_digest.clone(),
            policy_digest: preparation.policy_digest.clone(),
            platform: preparation.platform.clone(),
        }),
        kind: kind as i32,
        evidence: Some(RuntimeOperationEvidenceReference {
            evidence_id: "runtime-operation-json".into(),
            sha256: DIGEST.into(),
            size_bytes: 100,
            contract: operations::CONTRACT.into(),
        }),
    };
    let mut next = cursor.clone();
    next.preparation
        .as_mut()
        .ok_or("preparation")?
        .operation_observations
        .insert(execution, identity.clone());
    match kind {
        RuntimeOperationKind::Discovery => {
            next.pending.as_mut().ok_or("pending")?.phase = ImagePreparationPhase::Building;
        }
        RuntimeOperationKind::Trust => {
            next.trust.as_mut().ok_or("trust")?.state = "verified".into();
        }
        RuntimeOperationKind::Build => {
            let p = next.pending.as_mut().ok_or("pending")?;
            p.phase = ImagePreparationPhase::Publishing;
            p.candidate_image_id = Some(DIGEST.into());
        }
        RuntimeOperationKind::Publication => {
            next.pending.as_mut().ok_or("pending")?.phase =
                ImagePreparationPhase::PublicationReadback;
        }
        RuntimeOperationKind::Unspecified => return Err("kind".into()),
    }
    Ok((profile, cursor, next, identity, payload))
}

#[test]
fn every_runtime_flow_rolls_back_receipt_outbox_and_cursor_then_replays_after_restart() -> TestResult
{
    for kind in [
        RuntimeOperationKind::Discovery,
        RuntimeOperationKind::Trust,
        RuntimeOperationKind::Build,
        RuntimeOperationKind::Publication,
    ] {
        let dir = tempdir()?;
        let path = dir.path().join("operations.sqlite");
        let mut store = CleanStore::open(&path, 3)?;
        let (profile, cursor, next, identity, payload) = setup(&mut store, kind)?;
        store.receive_runtime_operation(&identity, &payload)?;
        let watermark = store.event_watermark()?;
        store.connection.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE ON reconciliation_cursors BEGIN SELECT RAISE(ABORT,'failed save');END;")?;
        assert!(
            store
                .save_runtime_operation_cursor(&profile, &next)
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(store.runtime_cursor(&profile)?)?,
            serde_json::to_value(&cursor)?
        );
        assert!(
            store
                .claim_runtime_publications(unix_seconds()?, 8)?
                .is_empty()
        );
        assert_eq!(store.event_watermark()?, watermark);
        store
            .connection
            .execute_batch("DROP TRIGGER fail_cursor;")?;
        store.save_runtime_operation_cursor(&profile, &next)?;
        let publications = store.claim_runtime_publications(unix_seconds()?, 8)?;
        assert_eq!(publications.len(), 1);
        let (_, fact) =
            ayni_orchestrator_events::operation_decisions::decode(&publications[0].bytes)?;
        assert_eq!(fact.observation, Some(payload.clone()));
        assert_eq!(fact.observation_id, identity.id);
        drop(store);
        let mut store = CleanStore::open(&path, 3)?;
        store.receive_runtime_operation(&identity, &payload)?;
        let watermark = store.event_watermark()?;
        store.save_runtime_operation_cursor(&profile, &next)?;
        assert_eq!(store.event_watermark()?, watermark);
        store.record_runtime_publication(&publications[0], None)?;
        let replay = store.claim_runtime_publications(unix_seconds()? + 300, 8)?;
        assert_eq!(replay, publications);
        store.record_runtime_publication(&replay[0], Some(1))?;
        let future = unix_seconds()? + 31 * 24 * 60 * 60;
        store.prune_runtime_operation_history(future)?;
        assert!(
            store
                .runtime_operation_receipt(
                    &payload.binding.as_ref().ok_or("binding")?.execution_id
                )?
                .is_some()
        );
        let mut drained = next;
        drained.preparation = None;
        drained.pending = None;
        drained.trust = None;
        store.save_runtime_cursor(&profile, &drained)?;
        store.prune_runtime_operation_history(future)?;
        assert!(
            store
                .runtime_operation_receipt(
                    &payload.binding.as_ref().ok_or("binding")?.execution_id
                )?
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn stale_cancelled_and_exhausted_runtime_results_cannot_advance() -> TestResult {
    for race in ["generation", "cancelled", "exhausted"] {
        let mut store = CleanStore::in_memory()?;
        let (profile, mut current, next, identity, payload) =
            setup(&mut store, RuntimeOperationKind::Build)?;
        store.receive_runtime_operation(&identity, &payload)?;
        match race {
            "generation" => {
                current
                    .preparation
                    .as_mut()
                    .ok_or("preparation")?
                    .generation = "replacement".into();
            }
            "cancelled" => current.pending.as_mut().ok_or("pending")?.failed = true,
            _ => current.failures = 3,
        }
        store.save_runtime_cursor(&profile, &current)?;
        let watermark = store.event_watermark()?;
        assert!(
            matches!(
                store.save_runtime_operation_cursor(&profile, &next),
                Err(CleanStoreError::StaleVersion)
            ),
            "{race}"
        );
        assert_eq!(store.event_watermark()?, watermark);
        assert!(
            store
                .claim_runtime_publications(unix_seconds()?, 8)?
                .is_empty()
        );
    }
    Ok(())
}
