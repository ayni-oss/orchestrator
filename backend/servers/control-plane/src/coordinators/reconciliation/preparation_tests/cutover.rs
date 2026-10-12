use super::*;
#[tokio::test]
async fn frozen_event_generation_does_not_fall_back_when_default_rolls_back() -> TestResult {
    use crate::profiles::ayni::preparation_state::VerificationAuthority;
    let mut fixture = Fixture::new().await?;
    let pending = fixture.cursor.pending.as_mut().ok_or("pending")?;
    pending.phase = ImagePreparationPhase::Verifying;
    pending.resolved_digest = Some(format!("sha256:{}", "b".repeat(64)));
    let name = pending.name.clone();
    fixture.cursor.preparation = Some(PreparationIdentity {
        operation_reporting: None,
        operation_observations: std::collections::BTreeMap::new(),
        verification_authority: VerificationAuthority::Events {
            reporting: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                source: "ayni://orchestrator/supervisors/frozen".into(),
                endpoint: "http://control-plane/events/verification".into(),
            },
        },
        generation: "frozen-generation".into(),
        source_revision: "a".repeat(40),
        policy_digest: format!("sha256:{}", "c".repeat(64)),
        lock_digest: format!("sha256:{}", "c".repeat(64)),
        contract_version: "0.15.0".into(),
        platform: "linux/arm64".into(),
        registry: vec![],
        certificate: None,
        runtime: fixture.runtime.clone(),
    });
    fixture
        .reconciler
        .save(&fixture.profile, &fixture.cursor)
        .await?;
    fixture.complete(&name, ExecutionState::Running);
    // Fixture configuration has no reporting and defaults to shadow: admitted
    // event work must still wait for its receipt rather than invoking observe.
    fixture.step().await?;
    assert_eq!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .verification_reads,
        0
    );
    fixture.complete(&name, ExecutionState::Failed);
    assert_eq!(fixture.step().await, Err("runtime_report_missing"));
    assert_eq!(fixture.cursor.failures, 0);
    assert_eq!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .verification_reads,
        0
    );
    Ok(())
}
