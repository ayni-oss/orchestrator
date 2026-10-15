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
    store.save_project_run_request(&run.id,&ProjectAgentRequest{run_id:run.id.clone(),project_id:c.project_id.clone(),model:run.model.clone(),history:String::new(),repositories:vec![ayni_orchestrator_runtime::project_agent::ResearchRepository{id:c.repository_connection_ids[0].clone(),repository_url:"https://github.com/ayni-oss/sandbox.git".into(),revision:REVISION.into(),coding_connection_id:String::new(),coding_model:run.model.clone(),runtime_profile_id:profile.id.clone()}],context:serde_json::json!({"repositories":[{"repository_id":c.repository_connection_ids[0],"profiles":[{"id":profile.id,"active_revision_id":profile.active_revision_id,"active_runtime_artifact_id":profile.active_runtime_artifact_id,"resolved_digest":artifact.resolved_digest}]}]}).to_string(),max_model_calls:24,deadline_seconds:600})?;
    store.complete_project_run(ProjectAgentResult {
        run_id: run.id.clone(),
        message: "Review this proposal".into(),
        model_calls: 2,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
        proposals: vec![ResearchProposal {
            repository_id: c.repository_connection_ids[0].clone(),
            revision: REVISION.into(),
            runtime_profile_id: profile.id.clone(),
            coding_connection_id: String::new(),
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
    let decision = v1::DecideProjectProposalRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    let approved = store.decide_project_proposal("approval-key", &decision)?;
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
    store.decide_project_proposal("approval-key", &decision)?;
    store.decide_project_proposal("new-approval-key", &decision)?;
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
    let decision = v1::DecideProjectProposalRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    assert!(matches!(
        store.decide_project_proposal("approval-key", &decision),
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
    let decision = v1::DecideProjectProposalRequest {
        idempotency_key: "reject".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "reject".into(),
    };
    let rejected = store.decide_project_proposal("reject-key", &decision)?;
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
fn accepted_attempt_outcomes_queue_one_summary_without_actions() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("message-key", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let c = store.project_conversation(&c.id)?;
    let decision = v1::DecideProjectProposalRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    let c = store.decide_project_proposal("approval-key", &decision)?;
    store.cancel_development_task(
        "cancel-key",
        &v1::CancelDevelopmentTaskRequest {
            idempotency_key: "cancel".into(),
            development_task_id: c.proposals[0].task_id.clone(),
        },
    )?;
    store.queue_project_outcome_summaries()?;
    store.queue_project_outcome_summaries()?;
    let queued = store.queued_project_runs()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1.purpose, "summary");
    let request = store
        .project_run_request(&queued[0].1.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    assert!(request.repositories.is_empty());
    assert_eq!(request.max_model_calls, 1);
    store.complete_project_run(ProjectAgentResult {
        run_id: queued[0].1.id.clone(),
        message: "Execution cancelled; no quality or publication result.".into(),
        model_calls: 1,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
        proposals: vec![],
    })?;
    store.queue_project_outcome_summaries()?;
    assert!(store.queued_project_runs()?.is_empty());
    assert_eq!(store.project_conversation(&c.id)?.proposals.len(), 1);
    Ok(())
}
#[test]
fn schema_eighteen_upgrade_preserves_existing_projects() -> Result<(), CleanStoreError> {
    let directory = tempdir().map_err(CleanStoreError::CreateDirectory)?;
    let path = directory.path().join("state.db");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, _, _) = ready_profile(&mut store)?;
    store.connection.execute_batch("DROP TABLE workspaces; DROP TABLE project_conversation_actions; DROP TABLE project_conversations; DROP TABLE project_agent_requests; DROP TABLE project_outcome_summaries; UPDATE clean_schema SET version=18;").map_err(CleanStoreError::Database)?;
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
fn repository_proposals_admit_independently_and_replay_partial_success()
-> Result<(), CleanStoreError> {
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
            run_id: run.id.clone(),
            project_id: c.project_id.clone(),
            model: run.model.clone(),
            history: String::new(),
            context,
            repositories,
            max_model_calls: 24,
            deadline_seconds: 600,
        },
    )?;
    let proposals = profiles
        .iter()
        .map(|(profile, _)| ResearchProposal {
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
    let first = v1::DecideProjectProposalRequest {
        idempotency_key: "first-approval".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    let approved = store.decide_project_proposal("first-approval", &first)?;
    store.observe_repository_snapshot(&second.id, SECOND_REVISION)?;
    let second_decision = v1::DecideProjectProposalRequest {
        proposal_id: c.proposals[1].id.clone(),
        idempotency_key: "second-approval".into(),
        ..first.clone()
    };
    assert!(matches!(
        store.decide_project_proposal("second-approval", &second_decision),
        Err(CleanStoreError::StaleVersion)
    ));
    assert_eq!(
        store
            .decide_project_proposal("first-approval", &first)?
            .proposals[0]
            .task_id,
        approved.proposals[0].task_id
    );
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
    let decision = v1::DecideProjectProposalRequest {
        idempotency_key: "approve".into(),
        conversation_id: c.id.clone(),
        proposal_id: c.proposals[0].id.clone(),
        decision: "approve".into(),
    };
    assert!(matches!(
        store.decide_project_proposal("approval-key", &decision),
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
