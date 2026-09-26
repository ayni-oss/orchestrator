use super::*;
use crate::preparation::{PreparationStatus, PrepareRepositoryInput};
use rusqlite::{Connection, params};

#[test]
fn opening_a_legacy_development_database_adds_current_task_columns() -> Result<(), StoreError> {
    let connection = Connection::open_in_memory().map_err(StoreError::Database)?;
    connection
        .execute_batch(
            "CREATE TABLE development_tasks (
                id TEXT PRIMARY KEY, repository_id TEXT NOT NULL, profile_id TEXT NOT NULL,
                repository_snapshot TEXT NOT NULL, profile_snapshot TEXT NOT NULL,
                start_revision TEXT NOT NULL, request TEXT NOT NULL,
                acceptance_criteria TEXT NOT NULL, phase TEXT NOT NULL,
                preparation_id TEXT, retry_of TEXT, requested_input TEXT,
                candidate_commit TEXT, validation_manifest TEXT,
                policy_or_test_edits INTEGER, draft_pr_url TEXT, failure TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );",
        )
        .map_err(StoreError::Database)?;
    let mut store = Store::from_connection(connection, 2)?;
    let columns = store
        .connection
        .prepare("PRAGMA table_info(development_tasks)")
        .map_err(StoreError::Database)?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(StoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::Database)?;
    assert!(columns.iter().any(|column| column == "deadline_seconds"));
    assert!(columns.iter().any(|column| column == "deadline_at"));
    assert!(columns.iter().any(|column| column == "protected_paths"));
    assert!(columns.iter().any(|column| column == "allow_test_edits"));
    assert!(columns.iter().any(|column| column == "scope_violations"));

    let repository = RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "sandbox".into(),
        installation_id: 1,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
    };
    let profile = AgentProfile {
        id: "other".into(),
        provider: "anthropic".into(),
        model: "example".into(),
        secret_ref: "other-key".into(),
    };
    let input = DevelopmentInput {
        repository_id: repository.id.clone(),
        profile_id: profile.id.clone(),
        start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
        request: "Fix the formula".into(),
        acceptance_criteria: vec!["3 -> 8".into()],
        change_constraints: ChangeConstraints::default(),
        deadline_seconds: None,
    };
    let id = TaskId::generate();
    let key = IdempotencyKey::try_from("legacy-submission".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    store
        .connection
        .execute(
            "INSERT INTO development_tasks(id, repository_id, profile_id, repository_snapshot,
             profile_snapshot, start_revision, request, acceptance_criteria, phase, created_at,
             updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'unsupported',1,1)",
            params![
                id.to_string(),
                input.repository_id,
                input.profile_id,
                serde_json::to_string(&repository)
                    .map_err(|_| StoreError::InvalidInput("could not serialize repository"))?,
                serde_json::to_string(&profile)
                    .map_err(|_| StoreError::InvalidInput("could not serialize profile"))?,
                input.start_revision,
                input.request,
                serde_json::to_string(&input.acceptance_criteria).map_err(|_| {
                    StoreError::InvalidInput("could not serialize acceptance criteria")
                })?,
            ],
        )
        .map_err(StoreError::Database)?;
    store
        .connection
        .execute(
            "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES (?1,?2,'submit',1)",
            params![key.as_str(), id.to_string()],
        )
        .map_err(StoreError::Database)?;
    store.migrate_development()?;
    let replay = store.submit_development(&key, &input)?;
    assert_eq!(replay.id, id);
    assert!(!replay.lifecycle_complete);
    assert_eq!(replay.lifecycle.len(), 1);
    assert_eq!(replay.lifecycle[0].stage, DevelopmentLifecycleStage::Queue);
    assert_eq!(
        replay.input.change_constraints,
        ChangeConstraints::default()
    );
    Ok(())
}

#[test]
fn opening_a_populated_legacy_database_marks_history_partial() -> Result<(), StoreError> {
    let connection = Connection::open_in_memory().map_err(StoreError::Database)?;
    connection
        .execute_batch(
            "CREATE TABLE development_tasks (
                id TEXT PRIMARY KEY, repository_id TEXT NOT NULL, profile_id TEXT NOT NULL,
                repository_snapshot TEXT NOT NULL, profile_snapshot TEXT NOT NULL,
                start_revision TEXT NOT NULL, request TEXT NOT NULL,
                acceptance_criteria TEXT NOT NULL, phase TEXT NOT NULL,
                preparation_id TEXT, retry_of TEXT, requested_input TEXT,
                candidate_commit TEXT, validation_manifest TEXT,
                policy_or_test_edits INTEGER, draft_pr_url TEXT, failure TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );",
        )
        .map_err(StoreError::Database)?;
    let id = TaskId::generate();
    let repository = RepositoryBinding {
        id: "trial".into(),
        owner: "ayni-oss".into(),
        repository: "sandbox".into(),
        installation_id: 1,
        base_branch: "main".into(),
        lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
    };
    let profile = AgentProfile {
        id: "coding".into(),
        provider: "openai".into(),
        model: "example".into(),
        secret_ref: "openai-api".into(),
    };
    connection.execute(
        "INSERT INTO development_tasks(id, repository_id, profile_id, repository_snapshot,
         profile_snapshot, start_revision, request, acceptance_criteria, phase, created_at,
         updated_at) VALUES (?1,'trial','coding',?2,?3,?4,'Fix it','[\"passes\"]','validating',1,7)",
        params![
            id.to_string(),
            serde_json::to_string(&repository)
                .map_err(|_| StoreError::InvalidInput("could not serialize repository"))?,
            serde_json::to_string(&profile)
                .map_err(|_| StoreError::InvalidInput("could not serialize profile"))?,
            "8a555ada2f6e3ee41de2485ed13da508beaebaeb",
        ],
    ).map_err(StoreError::Database)?;

    let store = Store::from_connection(connection, 2)?;
    let task = store.development_task(id)?;
    assert!(!task.lifecycle_complete);
    assert_eq!(task.lifecycle.len(), 1);
    assert_eq!(task.lifecycle[0].event_time, 7);
    assert_eq!(
        task.lifecycle[0].stage,
        DevelopmentLifecycleStage::Validation
    );
    assert_eq!(
        task.lifecycle[0].outcome,
        DevelopmentLifecycleOutcome::Started
    );
    Ok(())
}

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
        change_constraints: ChangeConstraints::default(),
        deadline_seconds: None,
    };
    let key = IdempotencyKey::try_from("development-submit".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    let first = store.submit_development(&key, &input)?;
    assert_eq!(first.phase, DevelopmentPhase::Unsupported);
    assert!(first.lifecycle_complete);
    assert_eq!(first, store.submit_development(&key, &input)?);
    for invalid_deadline in [0, 86_401] {
        let invalid = DevelopmentInput {
            deadline_seconds: Some(invalid_deadline),
            ..input.clone()
        };
        let invalid_key = IdempotencyKey::try_from(format!("invalid-{invalid_deadline}"))
            .map_err(StoreError::InvalidLifecycleData)?;
        assert!(matches!(
            store.submit_development(&invalid_key, &invalid),
            Err(StoreError::InvalidInput(
                "deadline seconds must be between 1 and 86400"
            ))
        ));
    }
    let invalid_scope = DevelopmentInput {
        change_constraints: ChangeConstraints {
            protected_paths: vec!["../outside".into()],
            allow_test_edits: false,
        },
        ..input
    };
    let invalid_scope_key = IdempotencyKey::try_from("invalid-scope".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    assert!(matches!(
        store.submit_development(&invalid_scope_key, &invalid_scope),
        Err(StoreError::InvalidChangeConstraints(_))
    ));
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
        change_constraints: ChangeConstraints::default(),
        deadline_seconds: Some(300),
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
    assert_eq!(task.input.deadline_seconds, Some(300));
    assert_eq!(task.deadline_at, Some(task.created_at + 300));
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
    assert!(matches!(
        store.cancel_development(task.id, &submit),
        Err(StoreError::ActionKeyReused(_))
    ));
    assert_eq!(
        store.cancel_development(task.id, &cancel)?.phase,
        DevelopmentPhase::Cancelled
    );
    assert_eq!(
        store.cancel_development(task.id, &cancel)?.lifecycle.len(),
        2
    );
    assert!(matches!(
        store.submit_development(&cancel, &task.input),
        Err(StoreError::ActionKeyReused(_))
    ));
    let cancelled = store.development_task(task.id)?;
    assert_eq!(
        cancelled.lifecycle[0].stage,
        DevelopmentLifecycleStage::Queue
    );
    assert_eq!(
        cancelled.lifecycle[1].stage,
        DevelopmentLifecycleStage::Queue
    );
    assert_eq!(
        cancelled.lifecycle[1].reason_code.as_deref(),
        Some("operator_cancelled")
    );
    assert!(!store.record_candidate(task.id, "8a555ada2f6e3ee41de2485ed13da508beaebaeb")?);
    Ok(())
}

fn assert_complete_lifecycle(task: &DevelopmentTask) {
    assert_eq!(
        task.lifecycle
            .iter()
            .map(|event| event.stage)
            .collect::<Vec<_>>(),
        vec![
            DevelopmentLifecycleStage::Queue,
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleStage::PrPreparation,
        ]
    );
    assert!(
        task.lifecycle
            .windows(2)
            .all(|events| events[0].event_time <= events[1].event_time)
    );
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
        change_constraints: ChangeConstraints::default(),
        deadline_seconds: Some(300),
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
    assert_complete_lifecycle(&done);
    Ok(())
}

fn assert_failed_with_reason(task: &DevelopmentTask, reason_code: &str) {
    assert_eq!(task.phase, DevelopmentPhase::Failed);
    assert_eq!(task.failure.as_deref(), Some(reason_code));
    assert_eq!(
        task.lifecycle
            .last()
            .and_then(|event| event.reason_code.as_deref()),
        Some(reason_code)
    );
}

#[test]
fn scope_rejection_is_durable_retryable_and_never_publishable() -> Result<(), StoreError> {
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
    let constraints = ChangeConstraints {
        protected_paths: vec!["scripts/oracle.sh".into()],
        allow_test_edits: false,
    };
    let task = store.submit_development(
        &IdempotencyKey::try_from("submit-scope-rejection".to_owned())
            .map_err(StoreError::InvalidLifecycleData)?,
        &DevelopmentInput {
            repository_id: "trial".into(),
            profile_id: "coding".into(),
            start_revision: revision.into(),
            request: "Fix the formula without changing the oracle".into(),
            acceptance_criteria: vec!["3 -> 8".into()],
            change_constraints: constraints.clone(),
            deadline_seconds: Some(300),
        },
    )?;
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::Queued,
        DevelopmentPhase::Coding
    )?);
    assert!(store.record_candidate(task.id, "4e67795c0cba21bd82c21bdd6c921417fbe29ff6")?);
    assert!(store.development_phase(
        task.id,
        DevelopmentPhase::ValidationQueued,
        DevelopmentPhase::Validating,
    )?);
    let violations = vec![
        "protected path \"scripts/oracle.sh\" covers changed path \"scripts/oracle.sh\"".into(),
    ];
    let oversized = vec!["violation".into(); ayni_orchestrator_domain::MAX_SCOPE_VIOLATIONS + 1];
    assert!(matches!(
        store.fail_validation(
            task.id,
            "scope_rejected",
            "scope_rejected",
            Some("sha256:scope-evidence"),
            Some(false),
            &oversized,
        ),
        Err(StoreError::InvalidInput(
            "scope violations exceed durable storage limits"
        ))
    ));
    assert_eq!(
        store.development_task(task.id)?.phase,
        DevelopmentPhase::Validating
    );
    assert!(store.fail_validation(
        task.id,
        "scope_rejected",
        "scope_rejected",
        Some("sha256:scope-evidence"),
        Some(false),
        &violations,
    )?);
    assert!(!store.fail_validation(
        task.id,
        "scope_rejected",
        "scope_rejected",
        Some("sha256:scope-evidence"),
        Some(false),
        &violations,
    )?);
    let rejected = store.development_task(task.id)?;
    assert_failed_with_reason(&rejected, "scope_rejected");
    assert_eq!(rejected.scope_violations, violations);
    assert!(
        store
            .begin_publication(task.id, "fix(example): repair formula", "body")
            .is_err()
    );

    let retry = store.retry_development(
        task.id,
        &IdempotencyKey::try_from("retry-scope-rejection".to_owned())
            .map_err(StoreError::InvalidLifecycleData)?,
        None,
    )?;
    assert_eq!(retry.retry_of, Some(task.id));
    assert_eq!(retry.input.change_constraints, constraints);
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
            change_constraints: ChangeConstraints {
                protected_paths: vec!["scripts/oracle.sh".into()],
                allow_test_edits: false,
            },
            deadline_seconds: Some(300),
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
    assert_eq!(follow_up.deadline_at, task.deadline_at);
    assert_eq!(follow_up.input.deadline_seconds, Some(300));
    assert_eq!(
        follow_up.input.change_constraints,
        task.input.change_constraints
    );
    assert!(follow_up.input.request.contains("Round down"));
    assert_eq!(
        store
            .retry_development(task.id, &key, Some("Round down"))?
            .id,
        follow_up.id
    );
    store
        .connection
        .execute(
            "UPDATE development_tasks SET deadline_at=0 WHERE id=?1",
            [task.id.to_string()],
        )
        .map_err(StoreError::Database)?;
    let expired_key = IdempotencyKey::try_from("provide-input-expired".to_owned())
        .map_err(StoreError::InvalidLifecycleData)?;
    assert!(matches!(
        store.retry_development(task.id, &expired_key, Some("Round up")),
        Err(StoreError::InvalidInput(
            "development task deadline has expired"
        ))
    ));
    Ok(())
}

#[test]
fn lifecycle_survives_restart_and_duplicate_reconciliation()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("orchestrator.sqlite");
    let revision = "8a555ada2f6e3ee41de2485ed13da508beaebaeb";
    let task_id = {
        let mut store = Store::open(&database, 3)?;
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
        mark_ready(&mut store, "trial", revision)?;
        let task = store.submit_development(
            &IdempotencyKey::try_from("restart-timeline".to_owned())?,
            &DevelopmentInput {
                repository_id: "trial".into(),
                profile_id: "coding".into(),
                start_revision: revision.into(),
                request: "Fix the formula".into(),
                acceptance_criteria: vec!["3 -> 8".into()],
                change_constraints: ChangeConstraints::default(),
                deadline_seconds: None,
            },
        )?;
        assert!(store.development_phase(
            task.id,
            DevelopmentPhase::Queued,
            DevelopmentPhase::Coding,
        )?);
        task.id
    };

    let mut reopened = Store::open(&database, 3)?;
    let task = reopened.development_task(task_id)?;
    assert!(task.lifecycle_complete);
    assert_eq!(task.lifecycle.len(), 2);
    assert_eq!(task.lifecycle[0].stage, DevelopmentLifecycleStage::Queue);
    assert_eq!(task.lifecycle[1].stage, DevelopmentLifecycleStage::Coding);
    assert!(!reopened.development_phase(
        task_id,
        DevelopmentPhase::Queued,
        DevelopmentPhase::Coding,
    )?);
    assert_eq!(reopened.development_task(task_id)?.lifecycle.len(), 2);
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
