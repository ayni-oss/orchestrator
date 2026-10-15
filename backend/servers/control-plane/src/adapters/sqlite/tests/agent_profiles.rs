use super::*;
use ayni_orchestrator_runtime::agent_profile::{Role, Settings};
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn configured() -> Result<
    (
        CleanStore,
        v1::ProjectConversation,
        v1::ProjectAgentRun,
        v1::RuntimeProfile,
    ),
    CleanStoreError,
> {
    let (mut store, c, mut run, profile) = setup()?;
    let provider = v1::Connection {
        id: "profiles-provider".into(),
        kind: "openai".into(),
        resource_version: 1,
        enabled: true,
        status: "ready".into(),
        observed_revision: "revision-1".into(),
        secret_name: "private-provider-secret".into(),
        secret_uid: "private-provider-uid".into(),
        ..Default::default()
    };
    store.save_connection(&provider)?;
    let project = store.project(&c.project_id)?;
    let workspace = store.workspace(&project.workspace_id)?;
    store.update_workspace(
        "profiles-defaults",
        &v1::UpdateWorkspaceRequest {
            idempotency_key: "profiles-defaults".into(),
            workspace_id: workspace.id,
            display_name: workspace.display_name,
            expected_resource_version: workspace.resource_version,
            agent_defaults: Some(v1::ProjectAgentDefaults {
                project_connection_id: provider.id.clone(),
                project_model: run.model.clone(),
                coding_connection_id: provider.id.clone(),
                coding_model: run.model.clone(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    run.connection_id = provider.id.clone();
    run.credential_revision = provider.observed_revision.clone();
    run.provider_credentials = store.freeze_task_provider(&provider.id)?;
    Ok((store, c, run, profile))
}
fn update_worker(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    settings: Option<Settings>,
) -> Result<(), CleanStoreError> {
    let project = store.project(&c.project_id)?;
    let workspace = store.workspace(&project.workspace_id)?;
    let mut defaults = workspace
        .agent_defaults
        .ok_or(CleanStoreError::CorruptState)?;
    defaults.worker_profile = settings.as_ref().map(crate::agent_profiles::wire_settings);
    store.update_workspace(
        &format!("profile-change-{}", workspace.resource_version),
        &v1::UpdateWorkspaceRequest {
            idempotency_key: format!("profile-change-{}", workspace.resource_version),
            workspace_id: workspace.id,
            display_name: workspace.display_name,
            expected_resource_version: workspace.resource_version,
            agent_defaults: Some(defaults),
            ..Default::default()
        },
    )?;
    Ok(())
}
#[test]
fn admitted_runs_and_tasks_keep_profile_revisions_across_settings_changes_and_restart() -> TestResult
{
    let (mut store, c, run, profile) = configured().map_err(|e| format!("configure: {e}"))?;
    let admitted = store.send_project_message("profile-message", &message(&c), run.clone())?;
    let original = admitted.runs[0]
        .planner_profile
        .clone()
        .ok_or("planner profile")?;
    let worker = admitted.runs[0]
        .worker_profile
        .clone()
        .ok_or("worker profile")?;
    assert!(
        !original
            .encode_to_vec()
            .windows(b"private-provider".len())
            .any(|w| w == b"private-provider")
    );
    complete(&mut store, &c, &run, &profile).map_err(|e| format!("complete: {e}"))?;
    let researched = store.project_conversation(&c.id)?;
    let mut changed = Settings::defaults(&Role::Worker);
    changed.instructions = "New instructions".into();
    update_worker(&mut store, &c, Some(changed)).map_err(|e| format!("change: {e}"))?;
    assert!(
        store
            .decide_project_plan("changed-approval", &plan_decision(&researched, "approve")?)
            .is_err()
    );
    assert_eq!(
        store.project_conversation(&c.id)?.runs[0]
            .worker_profile
            .as_ref(),
        Some(&worker)
    );
    update_worker(&mut store, &c, None).map_err(|e| format!("restore: {e}"))?;
    let accepted = store
        .decide_project_plan("original-approval", &plan_decision(&researched, "approve")?)
        .map_err(|e| format!("approve: {e}"))?;
    let task = store.development_task(&accepted.proposals[0].task_id)?;
    let frozen = task
        .snapshot
        .as_ref()
        .and_then(|s| s.agent_profile.as_ref())
        .ok_or("task profile")?;
    assert_eq!(frozen.settings, worker.settings);
    store.validate_durable_state()?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("profiles.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    let reopened = CleanStore::open(&path, 3).map_err(|e| format!("reopen: {e}"))?;
    assert_eq!(
        reopened.project_conversation(&c.id)?.runs[0]
            .planner_profile
            .as_ref(),
        Some(&original)
    );
    assert_eq!(reopened.development_task(&task.id)?.snapshot, task.snapshot);
    Ok(())
}
#[test]
fn invalid_and_broader_project_profiles_are_rejected_atomically() -> TestResult {
    let (mut store, c, _, _) = configured()?;
    let mut settings = Settings::defaults(&Role::Worker);
    settings.allowed_tools = vec!["ayni_check".into()];
    settings.max_model_calls = 8;
    update_worker(&mut store, &c, Some(settings))?;
    let project = store.project(&c.project_id)?;
    let mut defaults = store
        .effective_project_defaults(&project)?
        .ok_or("defaults")?;
    defaults.worker_profile = Some(crate::agent_profiles::wire_settings(&Settings::defaults(
        &Role::Worker,
    )));
    let request = v1::UpdateProjectRequest {
        idempotency_key: "profile-project-update".into(),
        project_id: project.id.clone(),
        expected_resource_version: project.resource_version,
        display_name: project.display_name.clone(),
        agent_defaults: Some(defaults.clone()),
        ..Default::default()
    };
    assert!(store.update_project("broader-profile", &request).is_err());
    defaults.planner_profile = Some(v1::AgentProfileSettings {
        allowed_tools: vec!["run_command".into()],
        max_model_calls: 8,
        deadline_seconds: 600,
        ..Default::default()
    });
    assert!(
        store
            .update_project(
                "invalid-profile",
                &v1::UpdateProjectRequest {
                    agent_defaults: Some(defaults),
                    ..request
                }
            )
            .is_err()
    );
    let foreign = v1::ProjectAgentDefaults {
        planner_profile: Some(crate::agent_profiles::wire_settings(&Settings {
            repository_ids: vec!["repository-foreign".into()],
            ..Settings::defaults(&Role::Planner)
        })),
        ..Default::default()
    };
    assert!(
        store
            .update_project(
                "foreign-profile",
                &v1::UpdateProjectRequest {
                    idempotency_key: "foreign-profile".into(),
                    project_id: project.id.clone(),
                    expected_resource_version: project.resource_version,
                    display_name: project.display_name.clone(),
                    agent_defaults: Some(foreign),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(store.project(&project.id)?, project);
    Ok(())
}
#[test]
fn revoked_provider_blocks_new_profile_admission_and_retained_profile_tampering_fails_closed()
-> TestResult {
    let (mut store, c, run, profile) = configured()?;
    let mut provider = store.get_connection(&run.connection_id)?;
    provider.enabled = false;
    store.save_connection(&provider)?;
    assert!(
        store
            .send_project_message("revoked-profile", &message(&c), run.clone())
            .is_err()
    );
    assert!(store.project_conversation(&c.id)?.runs.is_empty());
    provider.enabled = true;
    store.save_connection(&provider)?;
    store.send_project_message("enabled-profile", &message(&c), run.clone())?;
    complete(&mut store, &c, &run, &profile)?;
    let researched = store.project_conversation(&c.id)?;
    let accepted =
        store.decide_project_plan("profile-approval", &plan_decision(&researched, "approve")?)?;
    let mut task = store.development_task(&accepted.proposals[0].task_id)?;
    task.snapshot
        .as_mut()
        .and_then(|s| s.agent_profile.as_mut())
        .and_then(|p| p.settings.as_mut())
        .ok_or("profile")?
        .allowed_tools
        .push("publish_pr".into());
    store.connection.execute(
        "UPDATE development_tasks SET payload=?1 WHERE id=?2",
        params![task.encode_to_vec(), task.id],
    )?;
    assert!(matches!(
        store.validate_durable_state(),
        Err(CleanStoreError::CorruptState)
    ));
    Ok(())
}
