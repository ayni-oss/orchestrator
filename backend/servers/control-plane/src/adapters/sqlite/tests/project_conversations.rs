use super::*;
use ayni_orchestrator_runtime::project_agent::{
    ProjectAgentRequest, ProjectAgentResult, ResearchProposal,
};
fn setup() -> Result<
    (
        CleanStore,
        v1::ProjectConversation,
        v1::ProjectAgentRun,
        v1::RuntimeProfile,
    ),
    CleanStoreError,
> {
    let mut store = CleanStore::open(":memory:", 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let task = task_request(&store, &repository.id, &profile.id, "task")?
        .request
        .ok_or(CleanStoreError::CorruptState)?;
    let c = store.create_project_conversation(
        "conversation-key",
        &v1::CreateProjectConversationRequest {
            planner_idle_timeout_seconds: None,
            idempotency_key: "create".into(),
            project_id: task.project_id,
            title: "Research change".into(),
            repository_connection_ids: vec![repository.id],
        },
    )?;
    let run = v1::ProjectAgentRun {
        id: "run-research".into(),
        state: "queued".into(),
        purpose: "research".into(),
        model: "gpt-5.6-sol".into(),
        ..Default::default()
    };
    Ok((store, c, run, profile))
}
fn message(c: &v1::ProjectConversation) -> v1::SendProjectMessageRequest {
    v1::SendProjectMessageRequest {
        idempotency_key: "send".into(),
        conversation_id: c.id.clone(),
        content: "Implement the bounded change".into(),
    }
}
fn complete(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    run: &v1::ProjectAgentRun,
    profile: &v1::RuntimeProfile,
) -> Result<(), CleanStoreError> {
    let artifact = store.runtime_artifact(
        profile
            .active_runtime_artifact_id
            .as_deref()
            .ok_or(CleanStoreError::CorruptState)?,
    )?;
    let saved = store.project_conversation(&c.id)?;
    let saved_run = saved
        .runs
        .iter()
        .find(|r| r.id == run.id)
        .ok_or(CleanStoreError::CorruptState)?;
    let planner_profile = saved_run
        .planner_profile
        .as_ref()
        .map(crate::agent_profiles::frozen)
        .transpose()
        .map_err(|_| CleanStoreError::CorruptState)?;
    let worker_profile = saved_run
        .worker_profile
        .as_ref()
        .map(crate::agent_profiles::frozen)
        .transpose()
        .map_err(|_| CleanStoreError::CorruptState)?;
    let coding_connection = worker_profile
        .as_ref()
        .map_or_else(String::new, |p| p.connection_id.clone());
    let calls = planner_profile
        .as_ref()
        .map_or(24, |p| p.settings.max_model_calls.min(24));
    store.save_project_run_request(&run.id,&ProjectAgentRequest{
        worker_profile,
            profile: planner_profile,run_id:run.id.clone(),project_id:c.project_id.clone(),model:run.model.clone(),history:String::new(),conversation_context:serde_json::Value::Null,repositories:vec![ayni_orchestrator_runtime::project_agent::ResearchRepository{id:c.repository_connection_ids[0].clone(),repository_url:"https://github.com/ayni-oss/sandbox.git".into(),revision:REVISION.into(),coding_connection_id:coding_connection.clone(),coding_model:run.model.clone(),runtime_profile_id:profile.id.clone()}],context:serde_json::json!({"repositories":[{"repository_id":c.repository_connection_ids[0],"profiles":[{"id":profile.id,"active_revision_id":profile.active_revision_id,"active_runtime_artifact_id":profile.active_runtime_artifact_id,"resolved_digest":artifact.resolved_digest}]}]}).to_string(),max_model_calls:calls,deadline_seconds:600})?;
    store.complete_project_run(ProjectAgentResult {
        run_id: run.id.clone(),
        message: "Review this proposal".into(),
        model_calls: 2,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
        proposals: vec![ResearchProposal {
            findings: String::new(),
            scope: String::new(),
            dependency_environment_impact: String::new(),
            repository_id: c.repository_connection_ids[0].clone(),
            revision: REVISION.into(),
            runtime_profile_id: profile.id.clone(),
            coding_connection_id: coding_connection,
            coding_model: "gpt-5.6-sol".into(),
            request: "Make the bounded change".into(),
            acceptance_criteria: vec!["Tests pass".into()],
            allowed_paths: vec![],
            allow_test_edits: false,
            forbidden_paths: vec![],
            coding_turns: 12,
            deadline_seconds: 3600,
            action: "create_task".into(),
            target_task_id: String::new(),
        }],
    })
}
#[test]
fn messages_are_durable_and_serialized_with_idempotent_replay() -> Result<(), CleanStoreError> {
    let (mut store, c, run, _) = setup()?;
    let first = store.send_project_message("message-key", &message(&c), run.clone())?;
    let replay = store.send_project_message("message-key", &message(&c), run.clone())?;
    assert_eq!(first, replay);
    assert_eq!(replay.messages.len(), 1);
    assert!(
        store.list_project_conversations(&c.project_id)?[0]
            .messages
            .is_empty()
    );
    assert_eq!(store.project_conversation(&c.id)?.messages.len(), 1);
    let mut next = message(&c);
    next.content = "another request".into();
    assert!(matches!(
        store.send_project_message("another-key", &next, run),
        Err(CleanStoreError::InvalidTransition)
    ));
    Ok(())
}
#[test]
fn approval_admits_once_and_rejects_source_changes() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    assert_eq!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .len(),
        0
    );
    let decision = v1::DecideProjectPlanRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        plan_id: c
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .id
            .clone(),
        version: 1,
        decision: "approve".into(),
    };
    let approved = store.decide_project_plan("approval-key", &decision)?;
    assert_eq!(approved.proposals[0].state, "approved");
    assert!(!approved.proposals[0].task_id.is_empty());
    let admitted = store.development_task(&approved.proposals[0].task_id)?;
    assert!(
        !admitted
            .snapshot
            .and_then(|s| s.change_constraints)
            .ok_or(CleanStoreError::CorruptState)?
            .allow_test_edits
    );
    store.decide_project_plan("approval-key", &decision)?;
    store.decide_project_plan("new-approval-key", &decision)?;
    assert_eq!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .len(),
        1
    );
    Ok(())
}
#[test]
fn stale_proposals_do_not_admit_tasks() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    store.observe_repository_snapshot(&c.repository_connection_ids[0], SECOND_REVISION)?;
    let c = store.project_conversation(&c.id)?;
    let decision = v1::DecideProjectPlanRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        plan_id: c
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .id
            .clone(),
        version: 1,
        decision: "approve".into(),
    };
    assert!(matches!(
        store.decide_project_plan("approval-key", &decision),
        Err(CleanStoreError::StaleVersion)
    ));
    assert!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .is_empty()
    );
    Ok(())
}
#[test]
fn interrupted_runs_allow_new_explicit_messages() -> Result<(), CleanStoreError> {
    let (mut store, c, run, _) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    store.fail_project_run(&run.id, "worker_interrupted")?;
    let retry = v1::ProjectAgentRun {
        id: "run-retry".into(),
        ..run
    };
    assert_eq!(
        store
            .send_project_message("retry-key", &message(&c), retry)?
            .runs
            .len(),
        2
    );
    Ok(())
}

#[test]
fn rejected_proposal_never_admits_task() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let decision = v1::DecideProjectPlanRequest {
        idempotency_key: "reject".into(),
        conversation_id: c.id.clone(),
        plan_id: c
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .id
            .clone(),
        version: 1,
        decision: "reject".into(),
    };
    let rejected = store.decide_project_plan("reject-key", &decision)?;
    assert_eq!(rejected.proposals[0].state, "rejected");
    assert!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .is_empty()
    );
    Ok(())
}
#[test]
fn accepted_attempt_outcomes_return_once_without_starting_agent_actions()
-> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let decision = v1::DecideProjectPlanRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        plan_id: c
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .id
            .clone(),
        version: 1,
        decision: "approve".into(),
    };
    let c = store.decide_project_plan("approval-key", &decision)?;
    store.cancel_development_task(
        "cancel-key",
        &v1::CancelDevelopmentTaskRequest {
            idempotency_key: "cancel".into(),
            development_task_id: c.proposals[0].task_id.clone(),
        },
    )?;
    store.sync_project_worker_updates()?;
    let cursor = store.project_conversation(&c.id)?.update_cursor;
    store.sync_project_worker_updates()?;
    let updated = store.project_conversation(&c.id)?;
    assert_eq!(updated.update_cursor, cursor);
    assert_eq!(updated.worker_updates.len(), 1);
    assert_eq!(
        updated.worker_updates[0].state,
        v1::DevelopmentAttemptState::Cancelled as i32
    );
    assert!(store.queued_project_runs()?.is_empty());
    assert_eq!(updated.proposals.len(), 1);
    Ok(())
}
#[test]
fn schema_eighteen_upgrade_preserves_existing_projects() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("state.db");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, _, _) = ready_profile(&mut store)?;
    store.connection.execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; DROP TABLE workspaces; DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; UPDATE clean_schema SET version=18;").map_err(CleanStoreError::Database)?;
    drop(store);
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(store.schema_version()?, SCHEMA_VERSION);
    assert_eq!(
        store.repository_connection(&repository.id)?.id,
        repository.id
    );
    Ok(())
}

#[test]
fn whole_plan_approval_rolls_back_when_second_repository_is_stale() -> Result<(), CleanStoreError> {
    let (mut store, original, run, first_profile) = setup()?;
    let second =
        store.create_repository_connection("second-repo", &repository_request("second", "456"))?;
    let second_snapshot = store.observe_repository_snapshot(&second.id, REVISION)?;
    let (second_profile, second_revision) = store.create_runtime_profile(
        "second-profile",
        &profile_request("second-profile", &second.id, &second_snapshot.id),
    )?;
    let second_artifact = store.record_runtime_artifact(
        &second_profile.id,
        verified_artifact(&second_revision.id, DIGEST),
    )?;
    let second_profile = store.runtime_profile(&second_profile.id)?;
    let project = store.create_project(
        "multi-project",
        &v1::CreateProjectRequest {
            workspace_id: String::new(),
            goal: String::new(),
            idempotency_key: "multi".into(),
            display_name: "Multi repository".into(),
            repository_connection_id: String::new(),
            repository_connection_ids: vec![
                original.repository_connection_ids[0].clone(),
                second.id.clone(),
            ],
        },
    )?;
    let c = store.create_project_conversation(
        "multi-conversation",
        &v1::CreateProjectConversationRequest {
            planner_idle_timeout_seconds: None,
            idempotency_key: "multi-conversation".into(),
            project_id: project.id,
            title: "Independent repository changes".into(),
            repository_connection_ids: vec![],
        },
    )?;
    store.send_project_message("multi-message", &message(&c), run.clone())?;
    let first_artifact = store.runtime_artifact(
        first_profile
            .active_runtime_artifact_id
            .as_deref()
            .ok_or(CleanStoreError::CorruptState)?,
    )?;
    let profiles = [
        (&first_profile, &first_artifact),
        (&second_profile, &second_artifact),
    ];
    let repositories = profiles
        .iter()
        .map(
            |(profile, _)| ayni_orchestrator_runtime::project_agent::ResearchRepository {
                id: profile.repository_connection_id.clone(),
                repository_url: "https://github.com/ayni-oss/sandbox.git".into(),
                revision: REVISION.into(),
                coding_connection_id: String::new(),
                coding_model: run.model.clone(),
                runtime_profile_id: profile.id.clone(),
            },
        )
        .collect();
    let context=serde_json::json!({"repositories":profiles.iter().map(|(profile,artifact)|serde_json::json!({"repository_id":profile.repository_connection_id,"profiles":[{"id":profile.id,"active_revision_id":profile.active_revision_id,"active_runtime_artifact_id":profile.active_runtime_artifact_id,"resolved_digest":artifact.resolved_digest}]})).collect::<Vec<_>>()}).to_string();
    store.save_project_run_request(
        &run.id,
        &ProjectAgentRequest {
            worker_profile: None,
            profile: None,
            run_id: run.id.clone(),
            project_id: c.project_id.clone(),
            model: run.model.clone(),
            history: String::new(),
            conversation_context: serde_json::Value::Null,
            context,
            repositories,
            max_model_calls: 24,
            deadline_seconds: 600,
        },
    )?;
    let proposals = profiles
        .iter()
        .map(|(profile, _)| ResearchProposal {
            findings: String::new(),
            scope: String::new(),
            dependency_environment_impact: String::new(),
            repository_id: profile.repository_connection_id.clone(),
            revision: REVISION.into(),
            runtime_profile_id: profile.id.clone(),
            coding_connection_id: String::new(),
            coding_model: run.model.clone(),
            request: "Bounded independent change".into(),
            acceptance_criteria: vec!["Checks pass".into()],
            allowed_paths: vec![],
            allow_test_edits: true,
            forbidden_paths: vec![],
            coding_turns: 12,
            deadline_seconds: 3600,
            action: "create_task".into(),
            target_task_id: String::new(),
        })
        .collect();
    store.complete_project_run(ProjectAgentResult {
        run_id: run.id,
        message: "Two independent proposals".into(),
        proposals,
        model_calls: 2,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
    })?;
    let c = store.project_conversation(&c.id)?;
    let first = v1::DecideProjectPlanRequest {
        idempotency_key: "first-approval".into(),
        conversation_id: c.id.clone(),
        plan_id: c
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .id
            .clone(),
        version: 1,
        decision: "approve".into(),
    };
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let valid_path = directory.path().join("ready.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [valid_path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    store.observe_repository_snapshot(&second.id, SECOND_REVISION)?;
    assert!(matches!(
        store.decide_project_plan("first-approval", &first),
        Err(CleanStoreError::StaleVersion)
    ));
    assert!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .is_empty()
    );
    let c = store.project_conversation(&c.id)?;
    assert!(
        c.proposals
            .iter()
            .all(|p| p.state == "pending" && p.task_id.is_empty())
    );
    let mut store = CleanStore::open(&valid_path, 3)?;
    let approved = store.decide_project_plan("first-approval", &first)?;
    assert!(approved.proposals.iter().all(|p| p.state == "approved"));
    let replay = store.decide_project_plan("new-key", &first)?;
    assert_eq!(approved, replay);
    assert_eq!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .len(),
        2
    );

    Ok(())
}

#[test]
fn changed_runtime_identity_requires_a_new_approval() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let revision = profile
        .active_revision_id
        .as_deref()
        .ok_or(CleanStoreError::CorruptState)?;
    let previous = store.runtime_artifact(
        profile
            .active_runtime_artifact_id
            .as_deref()
            .ok_or(CleanStoreError::CorruptState)?,
    )?;
    store.transition_runtime_artifact(
        &previous.id,
        previous.resource_version,
        v1::RuntimeArtifactStatus::Unavailable,
        None,
    )?;
    store.record_runtime_artifact(&profile.id, verified_artifact(revision, SECOND_DIGEST))?;
    let c = store.project_conversation(&c.id)?;
    let decision = v1::DecideProjectPlanRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        plan_id: c
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .id
            .clone(),
        version: 1,
        decision: "approve".into(),
    };
    assert!(matches!(
        store.decide_project_plan("approval-key", &decision),
        Err(CleanStoreError::StaleVersion)
    ));
    assert!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .is_empty()
    );
    Ok(())
}

fn plan_decision(
    c: &v1::ProjectConversation,
    decision: &str,
) -> Result<v1::DecideProjectPlanRequest, CleanStoreError> {
    let p = c.plan.as_ref().ok_or(CleanStoreError::CorruptState)?;
    Ok(v1::DecideProjectPlanRequest {
        idempotency_key: decision.into(),
        conversation_id: c.id.clone(),
        plan_id: p.id.clone(),
        version: p
            .versions
            .last()
            .ok_or(CleanStoreError::CorruptState)?
            .version,
        decision: decision.into(),
    })
}
#[test]
fn revisions_keep_admitted_tasks_and_supersede_only_pending_versions() -> Result<(), CleanStoreError>
{
    let (mut store, c, mut run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let first = plan_decision(&c, "approve")?;
    let admitted = store.decide_project_plan("approve1", &first)?;
    let task = store.development_task(&admitted.proposals[0].task_id)?;
    run.id = "run-two".into();
    store.send_project_message("send2", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let second = store.project_conversation(&c.id)?;
    let prior = plan_decision(&second, "approve")?;
    run.id = "run-three".into();
    store.send_project_message("send3", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let third = store.project_conversation(&c.id)?;
    let versions = &third
        .plan
        .as_ref()
        .ok_or(CleanStoreError::CorruptState)?
        .versions;
    assert_eq!(
        versions
            .iter()
            .map(|v| v.state.as_str())
            .collect::<Vec<_>>(),
        vec!["approved", "superseded", "pending"]
    );
    assert_eq!(third.proposals[0], admitted.proposals[0]);
    assert_eq!(store.development_task(&task.id)?, task);
    assert!(matches!(
        store.decide_project_plan("stale", &prior),
        Err(CleanStoreError::StaleVersion)
    ));
    let replay = store.decide_project_plan("new-approve1", &first)?;
    assert_eq!(replay.proposals[0].task_id, task.id);
    let mut different = first.clone();
    different.version = 3;
    assert!(matches!(
        store.decide_project_plan("new-approve1", &different),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    let reject = plan_decision(&third, "reject")?;
    store.decide_project_plan("reject3", &reject)?;
    store.decide_project_plan("reject3", &reject)?;
    let mut conflict = reject;
    conflict.decision = "approve".into();
    assert!(matches!(
        store.decide_project_plan("reject3", &conflict),
        Err(CleanStoreError::IdempotencyConflict)
    ));
    assert_eq!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .len(),
        1
    );
    Ok(())
}
#[test]
fn legacy_create_decisions_require_exact_version_but_approved_receipts_replay()
-> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let legacy = v1::DecideProjectProposalRequest {
        idempotency_key: "legacy".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    assert!(matches!(
        store.decide_project_proposal("legacy", &legacy),
        Err(CleanStoreError::InvalidTransition)
    ));
    store.decide_project_plan("plan", &plan_decision(&c, "approve")?)?;
    assert_eq!(
        store.decide_project_proposal("legacy", &legacy)?.proposals[0].state,
        "approved"
    );
    Ok(())
}
fn remove_plan_representation(store: &mut CleanStore) -> Result<(), CleanStoreError> {
    let payloads = store
        .connection
        .prepare("SELECT payload FROM project_conversations")
        .map_err(CleanStoreError::Database)?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(CleanStoreError::Database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CleanStoreError::Database)?;
    for payload in payloads {
        let mut c: v1::ProjectConversation = decode(&payload)?;
        c.plan = None;
        store
            .connection
            .execute(
                "UPDATE project_conversations SET payload=?1 WHERE id=?2",
                params![c.encode_to_vec(), c.id],
            )
            .map_err(CleanStoreError::Database)?;
    }
    store
        .connection
        .execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; UPDATE clean_schema SET version=20")
        .map_err(CleanStoreError::Database)
}
#[test]
fn migration_preserves_history_and_receipts_across_restart() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let c = store.decide_project_plan("approved", &plan_decision(&c, "approve")?)?;
    let original = c.proposals.clone();
    let legacy = v1::DecideProjectProposalRequest {
        idempotency_key: "old-key".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    store.remember_conversation("old-key", &legacy, &c.id)?;
    remove_plan_representation(&mut store)?;
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("state.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(store.schema_version()?, super::super::SCHEMA_VERSION);
    let migrated = store.project_conversation(&c.id)?;
    assert_eq!(migrated.proposals, original);
    assert!(migrated.update_cursor > c.update_cursor);
    assert_eq!(
        migrated
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .versions[0]
            .state,
        "approved"
    );
    assert_eq!(
        store.decide_project_proposal("old-key", &legacy)?.proposals,
        original
    );
    drop(store);
    let store = CleanStore::open(&path, 3)?;
    assert_eq!(store.project_conversation(&c.id)?, migrated);
    Ok(())
}
#[test]
fn corrupt_legacy_history_rolls_back_entire_migration() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    remove_plan_representation(&mut store)?;
    let mut c = legacy_conversation(&store, &c.id)?;
    c.proposals[0].state = "unknown".into();
    store
        .connection
        .execute(
            "UPDATE project_conversations SET payload=?1 WHERE id=?2",
            params![c.encode_to_vec(), c.id],
        )
        .map_err(CleanStoreError::Database)?;
    assert!(super::super::project_plans::migrate(&store.connection).is_err());
    assert_eq!(store.schema_version()?, 20);
    assert!(legacy_conversation(&store, &c.id)?.plan.is_none());
    Ok(())
}
#[test]
fn migration_keeps_mixed_terminal_decisions_non_actionable() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let mut c = store.decide_project_plan("approved", &plan_decision(&c, "approve")?)?;
    let mut rejected = c.proposals[0].clone();
    rejected.id = "proposal-rejected".into();
    rejected.state = "rejected".into();
    rejected.task_id.clear();
    rejected.attempt_id.clear();
    c.proposals.push(rejected.clone());
    c.plan = None;
    store
        .connection
        .execute(
            "UPDATE project_conversations SET payload=?1 WHERE id=?2",
            params![c.encode_to_vec(), c.id],
        )
        .map_err(CleanStoreError::Database)?;
    store
        .connection
        .execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; UPDATE clean_schema SET version=20")
        .map_err(CleanStoreError::Database)?;
    super::super::project_plans::migrate(&store.connection)?;
    super::super::project_context::migrate(&store.connection)?;
    super::super::project_sessions::migrate(&store.connection)?;
    super::super::project_origins::migrate(&store.connection)?;
    let migrated = store.project_conversation(&c.id)?;
    assert_eq!(migrated.proposals, c.proposals);
    assert_eq!(
        migrated
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .versions[0]
            .state,
        "superseded"
    );
    assert!(matches!(
        store.decide_project_plan("mixed", &plan_decision(&migrated, "approve")?),
        Err(CleanStoreError::InvalidTransition)
    ));
    assert_eq!(store.project_conversation(&c.id)?.proposals[1], rejected);
    Ok(())
}

#[test]
fn separate_database_connections_serialize_duplicate_plan_approvals() -> Result<(), CleanStoreError>
{
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let command = plan_decision(&c, "approve")?;
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("state.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .map_err(CleanStoreError::Database)?;
    drop(store);
    std::thread::scope(|scope| -> Result<(), CleanStoreError> {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = vec![];
        for key in ["one", "two"] {
            let path = path.clone();
            let command = command.clone();
            let barrier = barrier.clone();
            handles.push(scope.spawn(
                move || -> Result<v1::ProjectConversation, CleanStoreError> {
                    let mut store = CleanStore::open(&path, 3)?;
                    barrier.wait();
                    store.decide_project_plan(key, &command)
                },
            ));
        }
        let first = handles
            .remove(0)
            .join()
            .map_err(|_| CleanStoreError::CorruptState)??;
        let second = handles
            .remove(0)
            .join()
            .map_err(|_| CleanStoreError::CorruptState)??;
        assert_eq!(first.proposals, second.proposals);
        Ok(())
    })?;
    let mut store = CleanStore::open(&path, 3)?;
    assert_eq!(
        store
            .list_development_tasks(Some(&c.project_id), None, None, None, None)?
            .items
            .len(),
        1
    );
    Ok(())
}

#[test]
#[ignore = "exports synthetic SQLite plan fixtures for disposable local Kubernetes validation"]
fn export_project_plan_fixture() -> Result<(), CleanStoreError> {
    let path =
        std::env::var("AYNI_PLAN_FIXTURE_PATH").map_err(|_| CleanStoreError::CorruptState)?;
    let legacy_path = std::env::var("AYNI_LEGACY_PLAN_FIXTURE_PATH")
        .map_err(|_| CleanStoreError::CorruptState)?;
    let (mut store, c, mut run, profile) = setup()?;
    store.send_project_message("fixture-send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let scope_key = |rpc: &str, key: &str| {
        format!(
            "request-{:x}",
            Sha256::digest(format!("ayni.idempotency.v1\0trusted-local\0{rpc}\0{key}").as_bytes())
        )
    };
    let mut first = plan_decision(&c, "approve")?;
    first.idempotency_key = "fixture-approve1".into();
    let approved =
        store.decide_project_plan(&scope_key("DecideProjectPlan", "fixture-approve1"), &first)?;
    let legacy = v1::DecideProjectProposalRequest {
        idempotency_key: "fixture-legacy-replay".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    store.remember_conversation(
        &scope_key("DecideProjectProposal", "fixture-legacy-replay"),
        &legacy,
        &c.id,
    )?;
    run.id = "run-fixture-two".into();
    store.send_project_message("fixture-send2", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    run.id = "run-fixture-three".into();
    store.send_project_message("fixture-send3", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let current = store.project_conversation(&c.id)?;
    store
        .connection
        .execute("VACUUM INTO ?1", [&path])
        .map_err(CleanStoreError::Database)?;
    remove_plan_representation(&mut store)?;
    store
        .connection
        .execute("VACUUM INTO ?1", [&legacy_path])
        .map_err(CleanStoreError::Database)?;
    let manifest = serde_json::json!({"conversationId":c.id,"projectId":c.project_id,"planId":current.plan.as_ref().ok_or(CleanStoreError::CorruptState)?.id,"pendingVersion":"3","approvedVersion":"1","supersededVersion":"2","approvedTaskId":approved.proposals[0].task_id,"legacyProposalId":legacy.proposal_id,"legacyIdempotencyKey":legacy.idempotency_key,"approvedIdempotencyKey":"fixture-approve1"});
    std::fs::write(
        format!("{path}.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|_| CleanStoreError::CorruptState)?,
    )
    .map_err(CleanStoreError::CreateDirectory)?;
    Ok(())
}

#[test]
fn maximum_metadata_includes_separators_and_survives_validation() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let mut c = store.project_conversation(&c.id)?;
    let proposal = c.proposals[0].clone();
    c.proposals = (0..100)
        .map(|index| v1::ProjectTaskProposal {
            id: format!("proposal-{index}"),
            ..proposal.clone()
        })
        .collect();
    c.plan = None;
    let metadata = vec![("x".repeat(16_384), String::new(), String::new()); 100];
    super::super::project_plans::append_version(&mut c, 0, &run.id, &metadata)?;
    super::super::project_plans::validate(&c)?;
    store.save_conversation(&mut c)?;
    store.validate_project_conversations()?;
    assert_eq!(
        c.plan.ok_or(CleanStoreError::CorruptState)?.versions[0]
            .findings
            .len(),
        100 * 16_384 + 198
    );
    Ok(())
}

#[test]
fn migration_preserves_more_than_one_hundred_twenty_eight_legacy_research_versions()
-> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("send1", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let mut c = store.project_conversation(&c.id)?;
    let proposal = c.proposals[0].clone();
    let run = c.runs[0].clone();
    c.runs = (0..129)
        .map(|index| v1::ProjectAgentRun {
            id: format!("run-{index}"),
            ..run.clone()
        })
        .collect();
    c.proposals = (0..129)
        .map(|index| v1::ProjectTaskProposal {
            id: format!("proposal-{index}"),
            run_id: format!("run-{index}"),
            ..proposal.clone()
        })
        .collect();
    c.plan = None;
    store
        .connection
        .execute(
            "UPDATE project_conversations SET payload=?1 WHERE id=?2",
            params![c.encode_to_vec(), c.id],
        )
        .map_err(CleanStoreError::Database)?;
    store
        .connection
        .execute_batch("DROP TABLE project_worker_updates; DROP TABLE project_task_origins; DROP TABLE project_planner_sessions; DROP TABLE project_context_summaries; DROP TABLE project_context_records; UPDATE clean_schema SET version=20")
        .map_err(CleanStoreError::Database)?;
    super::super::project_plans::migrate(&store.connection)?;
    super::super::project_context::migrate(&store.connection)?;
    super::super::project_sessions::migrate(&store.connection)?;
    super::super::project_origins::migrate(&store.connection)?;
    let migrated = store.project_conversation(&c.id)?;
    assert_eq!(migrated.proposals, c.proposals);
    let versions = migrated.plan.ok_or(CleanStoreError::CorruptState)?.versions;
    assert_eq!(versions.len(), 129);
    assert!(versions[..128].iter().all(|v| v.state == "superseded"));
    assert_eq!(versions[128].state, "pending");
    store.validate_project_conversations()?;
    Ok(())
}

#[path = "project_context_fixture.rs"]
mod context_fixture;

#[path = "project_context_lifecycle.rs"]
mod context_lifecycle;

#[path = "project_session_fixture.rs"]
mod session_fixture;

fn legacy_conversation(
    store: &CleanStore,
    id: &str,
) -> Result<v1::ProjectConversation, CleanStoreError> {
    get_payload(
        &store.connection,
        "SELECT payload FROM project_conversations WHERE id=?1",
        id,
        "project_conversation",
    )
}
#[path = "project_worker_evidence.rs"]
mod worker_evidence;

#[path = "agent_profiles.rs"]
mod agent_profiles;
