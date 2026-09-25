use super::*;
use crate::preparation::{PreparationStatus, PrepareRepositoryInput};

fn mark_ready(store: &mut Store, repository_id: &str, revision: &str) -> Result<(), StoreError> {
    let key = IdempotencyKey::try_from(format!("prepare-{repository_id}"))
        .map_err(StoreError::InvalidLifecycleData)?;
    let preparation = store.prepare_repository(
        repository_id,
        &key,
        &PrepareRepositoryInput {
            revision: revision.to_owned(),
        },
    )?;
    assert!(store.set_preparation_status(
        &preparation.id,
        PreparationStatus::Queued,
        PreparationStatus::Preparing,
    )?);
    assert!(store.complete_preparation(&preparation.id, 10)?);
    Ok(())
}

#[test]
fn submission_is_durable_and_unsupported_provider_is_explicit() -> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "sandbox".into(),
        installation_id: 1,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
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
    let key = IdempotencyKey::try_from("development-submit".to_owned())
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
        repository: "sandbox".into(),
        installation_id: 11,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
    })?;
    store.put_agent_profile(&AgentProfile {
        id: "coding".into(),
        provider: "openai".into(),
        model: "model-a".into(),
        secret_ref: "openai-api".into(),
    })?;
    let input = DevelopmentInput {
        repository_id: "trial".into(),
        profile_id: "coding".into(),
        start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
        request: "Fix the formula".into(),
        acceptance_criteria: vec!["3 -> 8".into()],
    };
    mark_ready(
        &mut store,
        "trial",
        "8a555ada2f6e3ee41de2485ed13da508beaebaeb",
    )?;
    let submit = IdempotencyKey::try_from("submit-development".to_owned())
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
    let cancel = IdempotencyKey::try_from("cancel-development".to_owned())
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
        repository: "sandbox".into(),
        installation_id: 11,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
    })?;
    store.put_agent_profile(&AgentProfile {
        id: "coding".into(),
        provider: "openai".into(),
        model: "model-a".into(),
        secret_ref: "openai-api".into(),
    })?;
    let input = DevelopmentInput {
        repository_id: "trial".into(),
        profile_id: "coding".into(),
        start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
        request: "Fix the formula".into(),
        acceptance_criteria: vec!["3 -> 8".into()],
    };
    mark_ready(
        &mut store,
        "trial",
        "8a555ada2f6e3ee41de2485ed13da508beaebaeb",
    )?;
    let submit = IdempotencyKey::try_from("submit-publication".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let task = store.submit_development(&submit, &input)?;
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::Queued,
        DevelopmentPhase::Coding
    )?);
    let candidate = "4e67795c0cba21bd82c21bdd6c921417fbe29ff6";
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
    assert_eq!(action.branch_name, format!("orchestrator/task-{}", task.id));
    assert_eq!(action.body, "first");
    store
        .connection
        .execute(
            "UPDATE publication_actions SET branch_name='existing/task-preserved' WHERE task_id=?1",
            [task.id.to_string()],
        )
        .map_err(StoreError::Database)?;
    let (_, action) = store.begin_publication(task.id, "fix(example): repair formula", "second")?;
    assert_eq!(action.branch_name, "existing/task-preserved");
    assert_eq!(action.body, "first");
    let changed_candidate = "9c876bca1b61229971900009f4ea9a81f2258909";
    assert!(!store.record_candidate(task.id, changed_candidate)?);
    assert!(
        store
            .complete_publication(
                task.id,
                changed_candidate,
                41,
                "https://github.com/ayni-oss/sandbox/pull/41",
            )
            .is_err()
    );
    assert_eq!(
        store.development_task(task.id)?.candidate_commit.as_deref(),
        Some(candidate)
    );
    store.complete_publication(
        task.id,
        candidate,
        42,
        "https://github.com/ayni-oss/sandbox/pull/42",
    )?;
    let (done, action) =
        store.begin_publication(task.id, "fix(example): repair formula", "third")?;
    assert_eq!(done.phase, DevelopmentPhase::Completed);
    assert_eq!(
        action.completed_url.as_deref(),
        Some("https://github.com/ayni-oss/sandbox/pull/42")
    );
    Ok(())
}

#[test]
fn requested_input_creates_a_linked_follow_up_task() -> Result<(), StoreError> {
    let mut store = Store::in_memory()?;
    store.put_repository_binding(&RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "sandbox".into(),
        installation_id: 11,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
    })?;
    store.put_agent_profile(&AgentProfile {
        id: "coding".into(),
        provider: "openai".into(),
        model: "model-a".into(),
        secret_ref: "openai-api".into(),
    })?;
    let revision = "8a555ada2f6e3ee41de2485ed13da508beaebaeb";
    mark_ready(&mut store, "trial", revision)?;
    let submit = IdempotencyKey::try_from("submit-input".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let task = store.submit_development(
        &submit,
        &DevelopmentInput {
            repository_id: "trial".into(),
            profile_id: "coding".into(),
            start_revision: revision.into(),
            request: "Fix the formula".into(),
            acceptance_criteria: vec!["3 -> 8".into()],
        },
    )?;
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::Queued,
        DevelopmentPhase::Coding,
    )?);
    assert!(store.record_requested_input(task.id, "Which rounding rule applies?")?);
    let key = IdempotencyKey::try_from("provide-input".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let follow_up = store.retry_development(task.id, &key, Some("Round down"))?;
    assert_eq!(follow_up.retry_of, Some(task.id));
    assert!(follow_up.input.request.contains("Round down"));
    assert_eq!(
        store
            .retry_development(task.id, &key, Some("Round down"))?
            .id,
        follow_up.id
    );
    Ok(())
}

#[test]
fn rejects_a_repository_outside_the_supported_sandbox() -> Result<(), StoreError> {
    let store = Store::in_memory()?;
    let error = store
        .put_repository_binding(&RepositoryBinding {
            id: "wrong-target".into(),
            owner: "ayni-oss".into(),
            repository: "orchestrator".into(),
            installation_id: 1,
            base_branch: "main".into(),
            lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
        })
        .err()
        .ok_or(StoreError::InvalidInput("non-sandbox target was accepted"))?;
    assert!(error.to_string().contains("ayni-oss/sandbox"));
    Ok(())
}
