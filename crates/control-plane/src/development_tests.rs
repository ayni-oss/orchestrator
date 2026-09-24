use super::*;

#[test]
fn submission_is_durable_and_unsupported_provider_is_explicit() -> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "orchestrator".into(),
        installation_id: 1,
        base_branch: "feat/request-to-draft-pr".into(),
        lock_sha256: "606972cd98e464c8a8befc5191b770539637a9ef3b872ea3ec52abae92928e4f".into(),
    })?;
    store.put_agent_profile(&AgentProfile {
        id: "other".into(),
        provider: "anthropic".into(),
        model: "example".into(),
        secret_ref: "other-key".into(),
    })?;
    let input = DevelopmentInput {
        repository_id: "trial".into(),
        profile_id: "other".into(),
        start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
        request: "Fix the formula".into(),
        acceptance_criteria: vec!["3 -> 8".into()],
    };
    let key = IdempotencyKey::try_from("m3-submit".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let first = store.submit_development(&key, &input)?;
    assert_eq!(first.phase, DevelopmentPhase::Unsupported);
    assert_eq!(first, store.submit_development(&key, &input)?);
    Ok(())
}

#[test]
fn snapshots_profile_and_blocks_stale_candidate_after_cancellation() -> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "orchestrator".into(),
        installation_id: 11,
        base_branch: "feat/request-to-draft-pr".into(),
        lock_sha256: "606972cd98e464c8a8befc5191b770539637a9ef3b872ea3ec52abae92928e4f".into(),
    })?;
    store.put_agent_profile(&AgentProfile {
        id: "coding".into(),
        provider: "openai".into(),
        model: "model-a".into(),
        secret_ref: "m3-openai".into(),
    })?;
    let input = DevelopmentInput {
        repository_id: "trial".into(),
        profile_id: "coding".into(),
        start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
        request: "Fix the formula".into(),
        acceptance_criteria: vec!["3 -> 8".into()],
    };
    let submit = IdempotencyKey::try_from("submit-m3".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let task = store.submit_development(&submit, &input)?;
    assert_eq!(task.phase, DevelopmentPhase::Queued);
    store.put_agent_profile(&AgentProfile {
        id: "coding".into(),
        provider: "anthropic".into(),
        model: "model-b".into(),
        secret_ref: "other".into(),
    })?;
    assert_eq!(
        store.development_task(task.id)?.agent_profile.model,
        "model-a"
    );
    let different = DevelopmentInput {
        request: "Different request".into(),
        ..input
    };
    assert!(matches!(
        store.submit_development(&submit, &different),
        Err(StoreError::ActionKeyReused(_))
    ));
    let cancel = IdempotencyKey::try_from("cancel-m3".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    assert_eq!(
        store.cancel_development(task.id, &cancel)?.phase,
        DevelopmentPhase::Cancelled
    );
    assert!(!store.record_candidate(task.id, "8a555ada2f6e3ee41de2485ed13da508beaebaeb")?);
    Ok(())
}

#[test]
fn publication_action_is_bound_to_one_candidate_and_reusable_after_completion()
-> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "orchestrator".into(),
        installation_id: 11,
        base_branch: "feat/request-to-draft-pr".into(),
        lock_sha256: "606972cd98e464c8a8befc5191b770539637a9ef3b872ea3ec52abae92928e4f".into(),
    })?;
    store.put_agent_profile(&AgentProfile {
        id: "coding".into(),
        provider: "openai".into(),
        model: "model-a".into(),
        secret_ref: "m3-openai".into(),
    })?;
    let input = DevelopmentInput {
        repository_id: "trial".into(),
        profile_id: "coding".into(),
        start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
        request: "Fix the formula".into(),
        acceptance_criteria: vec!["3 -> 8".into()],
    };
    let submit = IdempotencyKey::try_from("submit-publication".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let task = store.submit_development(&submit, &input)?;
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::Queued,
        DevelopmentPhase::Coding
    )?);
    let candidate = "42f2605d14aa980d14608a8d2f518bc05fcc47b6";
    assert!(store.record_candidate(task.id, candidate)?);
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::ValidationQueued,
        DevelopmentPhase::Validating
    )?);
    assert!(store.record_validation(task.id, "sha256:evidence", false)?);
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::PrQueued,
        DevelopmentPhase::PreparingPr
    )?);
    let (_, action) = store.begin_publication(task.id, "fix(example): repair formula", "first")?;
    assert_eq!(action.body, "first");
    let (_, action) = store.begin_publication(task.id, "fix(example): repair formula", "second")?;
    assert_eq!(action.body, "first");
    store.complete_publication(
        task.id,
        candidate,
        42,
        "https://github.com/ayni-oss/orchestrator/pull/42",
    )?;
    let (done, action) =
        store.begin_publication(task.id, "fix(example): repair formula", "third")?;
    assert_eq!(done.phase, DevelopmentPhase::Completed);
    assert_eq!(
        action.completed_url.as_deref(),
        Some("https://github.com/ayni-oss/orchestrator/pull/42")
    );
    Ok(())
}
