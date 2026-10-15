use super::*;
use ayni_orchestrator_runtime::project_agent::{
    ProjectAgentRequest, ProjectAgentResult, ResearchProposal, ResearchRepository,
};
use v1::project_conversation_service_client::ProjectConversationServiceClient;

const CONNECTION: &str = "synthetic-coding-connection";
const MODEL: &str = "gpt-5.6-sol";

async fn start_with_source(path: &Path) -> TestResult<(Server, Channel)> {
    let mut state = AppState::open(path, 3)?;
    state.source = Some(std::sync::Arc::new(
        crate::adapters::github::SourceClient::fixed(SOURCE)?,
    ));
    with_store(&state, |store| {
        store.save_connection(&v1::Connection {
            id: CONNECTION.into(),
            kind: "openai".into(),
            management_source: "ui".into(),
            secret_name: "synthetic-test-secret-reference".into(),
            secret_uid: "synthetic-test-uid".into(),
            observed_revision: "synthetic-test-revision".into(),
            status: "ready".into(),
            enabled: true,
            resource_version: 1,
            ..Default::default()
        })
    })
    .await?;
    state.provider_catalogs.lock().await.insert(
        format!("{CONNECTION}:synthetic-test-uid:synthetic-test-revision"),
        crate::ModelCatalog::fixed(vec![crate::AvailableModel {
            id: MODEL.into(),
            name: None,
            owned_by: None,
            created_at: None,
            context_length: None,
            max_output_tokens: None,
        }]),
    );
    Server::start_state(state).await
}

async fn research_plan(
    server: &Server,
    channel: Channel,
    profiles: &[(&RuntimeFixture, &v1::RuntimeArtifact)],
) -> TestResult<v1::ProjectConversation> {
    let conversation = ProjectConversationServiceClient::new(channel)
        .create_project_conversation(v1::CreateProjectConversationRequest {
            idempotency_key: "conversation".into(),
            project_id: profiles[0].0.project.id.clone(),
            title: "Review the exact task set".into(),
            repository_connection_ids: profiles
                .iter()
                .map(|(p, _)| p.repository.id.clone())
                .collect(),
        })
        .await?
        .into_inner()
        .conversation
        .ok_or("conversation missing")?;
    let inputs = profiles
        .iter()
        .map(|(profile, artifact)| {
            (
                profile.repository.id.clone(),
                profile.profile.id.clone(),
                profile.revision.id.clone(),
                (*artifact).clone(),
            )
        })
        .collect::<Vec<_>>();
    Ok(with_store(&server.state, move |store| {
        // Research execution is a fixture; approvals still traverse the real
        // tonic service, source/provider checks, and durable admission boundary.
        let run = v1::ProjectAgentRun {
            id: "synthetic-research-run".into(),
            state: "queued".into(),
            purpose: "research".into(),
            model: MODEL.into(),
            ..Default::default()
        };
        store.send_project_message(
            "research-message",
            &v1::SendProjectMessageRequest {
                idempotency_key: "research-message".into(),
                conversation_id: conversation.id.clone(),
                content: "Make two bounded changes".into(),
            },
            run.clone(),
        )?;
        store.save_project_run_request(
            &run.id,
            &ProjectAgentRequest {
                run_id: run.id.clone(),
                project_id: conversation.project_id.clone(),
                model: MODEL.into(),
                history: String::new(),
            conversation_context: serde_json::Value::Null,
                repositories: inputs.iter().map(|(repository_id, profile_id, _, _)| ResearchRepository {
                    id: repository_id.clone(),
                    repository_url: "https://github.com/test-org/repository.git".into(),
                    revision: SOURCE.into(),
                    coding_connection_id: CONNECTION.into(),
                    coding_model: MODEL.into(),
                    runtime_profile_id: profile_id.clone(),
                }).collect(),
                context: serde_json::json!({"repositories": inputs.iter().map(|(repository_id, profile_id, revision_id, artifact)| serde_json::json!({
                    "repository_id": repository_id,
                    "profiles": [{"id": profile_id, "active_revision_id": revision_id,
                        "active_runtime_artifact_id": artifact.id,
                        "resolved_digest": artifact.resolved_digest}]
                })).collect::<Vec<_>>()}).to_string(),
                max_model_calls: 24,
                deadline_seconds: 600,
            },
        )?;
        store.complete_project_run(ProjectAgentResult {
            run_id: run.id,
            message: "Review both changes before admission".into(),
            model_calls: 2,
            token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
            proposals: inputs.iter().zip(["Update the first example", "Update the second example"])
                .map(|((repository_id, profile_id, _, _), request)| ResearchProposal {
                    findings: "Both examples need updating".into(),
                    scope: "Documentation examples".into(),
                    dependency_environment_impact: "No environment change required".into(),
                    repository_id: repository_id.clone(),
                    revision: SOURCE.into(),
                    runtime_profile_id: profile_id.clone(),
                    coding_connection_id: CONNECTION.into(),
                    coding_model: MODEL.into(),
                    request: request.into(),
                    acceptance_criteria: vec![format!("{request} matches current behavior")],
                    allowed_paths: vec!["docs".into()],
                    allow_test_edits: false,
                    forbidden_paths: vec!["policy".into()],
                    coding_turns: 12,
                    deadline_seconds: 3600,
                    action: "create_task".into(),
                    target_task_id: String::new(),
                })
                .collect(),
        })?;
        store.project_conversation(&conversation.id)
    })
    .await?)
}

#[tokio::test]
async fn concurrent_plan_approvals_admit_exactly_one_reviewed_task_set_and_replay_after_restart()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("state.sqlite");
    let (server, channel) = start_with_source(&path).await?;
    let mut profile = create_profile(&server, channel.clone()).await?;
    let second_profile = create_profile_named(&server, channel.clone(), "second-", "456").await?;
    let artifact =
        observe_verified_runtime(&server, &profile, &format!("sha256:{}", "a".repeat(64))).await?;
    let second_artifact = observe_verified_runtime(
        &server,
        &second_profile,
        &format!("sha256:{}", "b".repeat(64)),
    )
    .await?;
    profile.project = v1::project_service_client::ProjectServiceClient::new(channel.clone())
        .create_project(v1::CreateProjectRequest {
            idempotency_key: "combined-project".into(),
            display_name: "Two repository plan".into(),
            repository_connection_ids: vec![
                profile.repository.id.clone(),
                second_profile.repository.id.clone(),
            ],
            ..Default::default()
        })
        .await?
        .into_inner()
        .project
        .ok_or("combined project missing")?;
    let reviewed = research_plan(
        &server,
        channel.clone(),
        &[(&profile, &artifact), (&second_profile, &second_artifact)],
    )
    .await?;
    let plan = reviewed.plan.as_ref().ok_or("plan missing")?;
    assert_eq!(plan.versions.len(), 1);
    assert_eq!(plan.versions[0].state, "pending");
    assert_eq!(plan.versions[0].proposal_ids.len(), 2);
    let first = v1::DecideProjectPlanRequest {
        idempotency_key: "first-concurrent-approval".into(),
        conversation_id: reviewed.id.clone(),
        plan_id: plan.id.clone(),
        version: plan.versions[0].version,
        decision: "approve".into(),
    };
    let second = v1::DecideProjectPlanRequest {
        idempotency_key: "second-concurrent-approval".into(),
        ..first.clone()
    };
    let mut first_client = ProjectConversationServiceClient::new(channel.clone());
    let mut second_client = ProjectConversationServiceClient::new(channel.clone());
    let (one, two) = tokio::join!(
        first_client.decide_project_plan(first.clone()),
        second_client.decide_project_plan(second.clone())
    );
    let approved = one?.into_inner().conversation.ok_or("approval missing")?;
    assert_eq!(two?.into_inner().conversation, Some(approved.clone()));
    assert_eq!(approved.proposals.len(), reviewed.proposals.len());
    let mut tasks = TaskClient::new(channel);
    let listed = tasks
        .list_development_tasks(v1::ListDevelopmentTasksRequest {
            project_id: Some(profile.project.id.clone()),
            ..Default::default()
        })
        .await?
        .into_inner()
        .development_tasks;
    assert_reviewed_tasks(&approved, &reviewed, &listed)?;
    server.stop().await?;
    // Receipt replay must survive restart without source/provider configuration.
    let (restarted, channel) = Server::start(&path).await?;
    let mut client = ProjectConversationServiceClient::new(channel.clone());
    for request in [first, second] {
        assert_eq!(
            client
                .decide_project_plan(request)
                .await?
                .into_inner()
                .conversation,
            Some(approved.clone())
        );
    }
    let after_restart = TaskClient::new(channel)
        .list_development_tasks(v1::ListDevelopmentTasksRequest {
            project_id: Some(profile.project.id),
            ..Default::default()
        })
        .await?
        .into_inner()
        .development_tasks;
    assert_eq!(after_restart, listed);
    restarted.stop().await
}

fn assert_reviewed_tasks(
    approved: &v1::ProjectConversation,
    reviewed: &v1::ProjectConversation,
    listed: &[v1::DevelopmentTask],
) -> TestResult {
    assert_eq!(listed.len(), 2);
    for (proposal, review) in approved.proposals.iter().zip(&reviewed.proposals) {
        assert_eq!(proposal.state, "approved");
        assert_eq!(proposal.request, review.request);
        let task = listed
            .iter()
            .find(|task| task.id == proposal.task_id)
            .ok_or("task missing")?;
        assert!(!proposal.attempt_id.is_empty());
        assert_eq!(task.active_attempt_id.as_ref(), Some(&proposal.attempt_id));
        assert_eq!(task.attempt_count, 1);
        let snapshot = task.snapshot.as_ref().ok_or("snapshot missing")?;
        let request = review.request.as_ref().ok_or("reviewed request missing")?;
        assert_eq!(snapshot.request, request.request);
        assert_eq!(snapshot.acceptance_criteria, request.acceptance_criteria);
        assert_eq!(snapshot.change_constraints, request.change_constraints);
        assert_eq!(snapshot.agent, request.agent);
        let runtime = snapshot.runtime.as_ref().ok_or("runtime missing")?;
        let researched = review
            .researched_runtime
            .as_ref()
            .ok_or("reviewed runtime missing")?;
        assert_eq!(runtime.runtime_profile_id, researched.runtime_profile_id);
        assert_eq!(
            runtime.runtime_profile_revision_id,
            researched.runtime_profile_revision_id
        );
        assert_eq!(runtime.runtime_artifact_id, researched.runtime_artifact_id);
        assert_eq!(runtime.resolved_digest, researched.resolved_digest);
        assert_eq!(
            snapshot
                .repository
                .as_ref()
                .ok_or("source missing")?
                .revision,
            request.repository_revision
        );
        assert_eq!(
            snapshot
                .limits
                .as_ref()
                .ok_or("limits missing")?
                .coding_turns,
            request.coding_turns.ok_or("coding turns missing")?
        );
        assert_eq!(
            snapshot
                .provider_credentials
                .as_ref()
                .ok_or("provider missing")?
                .connection_id,
            CONNECTION
        );
    }
    Ok(())
}

#[tokio::test]
async fn conversation_archive_transport_preserves_old_context_and_rejects_project_mismatch()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("context.sqlite");
    let (server, channel) = start_with_source(&path).await?;
    let profile = create_profile(&server, channel.clone()).await?;
    let mut client = ProjectConversationServiceClient::new(channel);
    let c = client
        .create_project_conversation(v1::CreateProjectConversationRequest {
            idempotency_key: "archive-transport".into(),
            project_id: profile.project.id.clone(),
            title: "Read old decisions".into(),
            repository_connection_ids: vec![profile.repository.id.clone()],
        })
        .await?
        .into_inner()
        .conversation
        .ok_or("missing conversation")?;
    let id = c.id.clone();
    with_store(&server.state, move |store| {
        let mut c = store.project_conversation(&id)?;
        c.messages = (0..600)
            .map(|i| v1::ProjectMessage {
                id: format!("transport-{i}"),
                role: "user".into(),
                content: if i == 0 {
                    "Keep the lavender decision".into()
                } else {
                    format!("Other topic {i}")
                },
                ..Default::default()
            })
            .collect();
        store.save_conversation(&mut c)
    })
    .await?;
    let request = v1::ListProjectConversationHistoryRequest {
        project_id: c.project_id.clone(),
        conversation_id: c.id.clone(),
        query: "lavender".into(),
        limit: 10,
        ..Default::default()
    };
    let result = client
        .list_project_conversation_history(request.clone())
        .await?
        .into_inner();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].source_id, "transport-0");
    let context = client
        .get_project_conversation_context(v1::GetProjectConversationContextRequest {
            project_id: c.project_id.clone(),
            conversation_id: c.id.clone(),
        })
        .await?
        .into_inner();
    assert_eq!(context.archive_record_count, 600);
    assert_eq!(context.recent_message_limit, 16);
    let mismatch = client
        .list_project_conversation_history(v1::ListProjectConversationHistoryRequest {
            project_id: "different-project".into(),
            ..request.clone()
        })
        .await
        .err()
        .ok_or("project mismatch must fail")?;
    assert_eq!(mismatch.code(), tonic::Code::FailedPrecondition);
    drop(client);
    server.stop().await?;
    let (server, channel) = start_with_source(&path).await?;
    let actual = ProjectConversationServiceClient::new(channel)
        .list_project_conversation_history(request)
        .await?
        .into_inner();
    assert_eq!(actual, result);
    server.stop().await?;
    Ok(())
}
