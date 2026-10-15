//! Synthetic, stopped-SQLite resident-session fixtures for the disposable local gate.
use super::*;
#[test]
#[ignore = "exports or mutates synthetic resident-session fixtures for Docker Desktop validation"]
fn export_project_session_fixture() -> Result<(), CleanStoreError> {
    let path = std::path::PathBuf::from(
        std::env::var("AYNI_SESSION_FIXTURE_PATH").map_err(|_| CleanStoreError::CorruptState)?,
    );
    let mode = std::env::var("AYNI_SESSION_FIXTURE_MODE").unwrap_or_else(|_| "seed".into());
    let manifest_path = path.with_extension("json");
    let (mut store, conversation_id) = if mode == "seed" {
        let (store, c, _, _) = setup()?;
        (store, c.id)
    } else {
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&manifest_path).map_err(CleanStoreError::CreateDirectory)?,
        )
        .map_err(|_| CleanStoreError::CorruptState)?;
        (
            CleanStore::open(&path, 3)?,
            manifest["conversationId"]
                .as_str()
                .ok_or(CleanStoreError::CorruptState)?
                .into(),
        )
    };
    let mut c = store.project_conversation(&conversation_id)?;
    let project = store.project(&c.project_id)?;
    let mut workspace = store.workspace(&project.workspace_id)?;
    let connection_id = "session-fixture-connection";
    let model = "session-fixture-model";
    let binding = fixture_binding(&c, connection_id);
    store.save_connection(&v1::Connection {
        id: connection_id.into(),
        kind: "openai".into(),
        management_source: "ui".into(),
        secret_name: binding.secret_name.clone(),
        secret_uid: binding.secret_uid.clone(),
        observed_revision: binding.credential_revision.clone(),
        status: "ready".into(),
        enabled: true,
        resource_version: 1,
        ..Default::default()
    })?;
    let planner_profile = fixture_planner_profile(&workspace, &mode);
    workspace.agent_defaults = Some(v1::ProjectAgentDefaults {
        planner_profile,
        worker_profile: None,
        project_connection_id: connection_id.into(),
        project_model: model.into(),
        coding_connection_id: connection_id.into(),
        coding_model: model.into(),
    });
    store
        .connection
        .execute(
            "UPDATE workspaces SET payload=?1 WHERE id=?2",
            params![workspace.encode_to_vec(), workspace.id],
        )
        .map_err(CleanStoreError::Database)?;
    if mode == "profiles" {
        // Settings changed; admitted turns retain their original frozen profile.
    } else if mode == "requeue" {
        requeue_bound_turn(&mut store, &mut c)?;
    } else if mode == "expire" {
        expire_fixture(&store, &c)?;
    } else if matches!(mode.as_str(), "seed" | "next" | "rotate") {
        let run_id = format!("session-fixture-run-{}", c.runs.len() + 1);
        let message = v1::SendProjectMessageRequest {
            idempotency_key: run_id.clone(),
            conversation_id: c.id.clone(),
            content: format!(
                "Research this synthetic planner question {}",
                c.runs.len() + 1
            ),
        };
        c = store.send_project_message(
            &run_id,
            &message,
            v1::ProjectAgentRun {
                id: run_id.clone(),
                state: "queued".into(),
                purpose: "research".into(),
                model: model.into(),
                connection_id: connection_id.into(),
                credential_revision: binding.credential_revision.clone(),
                provider_credentials: Some(binding),
                session_managed: true,
                created_at: Some(timestamp(unix_seconds()?)),
                ..Default::default()
            },
        )?;
        persist_fixture_turn_request(&mut store, &c, &run_id, model)?;
    } else {
        return Err(CleanStoreError::InvalidTransition);
    }
    let session = store.planner_session(&c.id)?;
    let run_id = c.runs.last().map_or("", |r| r.id.as_str());
    let session_id = session.as_ref().map_or("", |s| s.session_id.as_str());
    let pod_name = if session_id.is_empty() {
        String::new()
    } else {
        format!(
            "project-{}",
            &format!("{:x}", Sha256::digest(session_id.as_bytes()))[..40]
        )
    };
    let manifest = serde_json::json!({"conversationId":c.id,"projectId":c.project_id,"workspaceId":workspace.id,"runId":run_id,"connectionId":connection_id,"model":model,"sessionId":session_id,"generation":session.as_ref().map(|s|s.generation),"sessionPodName":pod_name});
    if mode == "seed" {
        store
            .connection
            .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
            .map_err(CleanStoreError::Database)?;
    }
    drop(store);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|_| CleanStoreError::CorruptState)?,
    )
    .map_err(CleanStoreError::CreateDirectory)?;
    Ok(())
}

fn requeue_bound_turn(
    store: &mut CleanStore,
    c: &mut v1::ProjectConversation,
) -> Result<(), CleanStoreError> {
    let run = c.runs.last_mut().ok_or(CleanStoreError::CorruptState)?;
    if run.state != "running" || run.session_id.is_empty() || run.session_deadline_at.is_none() {
        return Err(CleanStoreError::InvalidTransition);
    }
    run.state = "queued".into();
    store.save_conversation(c)
}

fn fixture_binding(
    c: &v1::ProjectConversation,
    connection_id: &str,
) -> v1::FrozenProviderCredentials {
    let prior = c.runs.last().and_then(|r| r.provider_credentials.as_ref());
    v1::FrozenProviderCredentials {
        connection_id: connection_id.into(),
        secret_name: std::env::var("AYNI_SESSION_MODEL_SECRET")
            .ok()
            .or_else(|| prior.map(|p| p.secret_name.clone()))
            .unwrap_or_else(|| "session-fixture-model".into()),
        secret_uid: std::env::var("AYNI_SESSION_MODEL_UID")
            .ok()
            .or_else(|| prior.map(|p| p.secret_uid.clone()))
            .unwrap_or_else(|| "session-fixture-uid".into()),
        credential_revision: std::env::var("AYNI_SESSION_MODEL_REVISION")
            .ok()
            .or_else(|| prior.map(|p| p.credential_revision.clone()))
            .unwrap_or_else(|| "1".into()),
    }
}

fn expire_fixture(store: &CleanStore, c: &v1::ProjectConversation) -> Result<(), CleanStoreError> {
    let now = unix_seconds()?;
    let started = now - i64::from(c.effective_planner_idle_timeout_seconds) - 1;
    let state = store
        .planner_session(&c.id)?
        .ok_or(CleanStoreError::CorruptState)?;
    if state.state != "idle" {
        return Err(CleanStoreError::InvalidTransition);
    }
    store.connection.execute("UPDATE project_planner_sessions SET payload=json_set(payload,'$.idle_started_at',?1,'$.last_activity_at',?1,'$.idle_expires_at',?2) WHERE conversation_id=?3",params![started,now-1,c.id]).map_err(CleanStoreError::Database)?;
    Ok(())
}

fn fixture_planner_profile(
    workspace: &v1::Workspace,
    mode: &str,
) -> Option<v1::AgentProfileSettings> {
    let profile_validation = std::env::var("AYNI_SESSION_FIXTURE_PROFILES").is_ok();
    if profile_validation {
        let mut selected = workspace
            .agent_defaults
            .as_ref()
            .and_then(|d| d.planner_profile.clone())
            .unwrap_or_else(|| v1::AgentProfileSettings {
                instructions: "Profile fixture revision 1".into(),
                allowed_tools: vec!["inspect_project_context".into()],
                skills: vec![v1::AgentSkillRevision {
                    id: "review".into(),
                    revision: "v1".into(),
                    instructions: "Context only; no additional permissions".into(),
                }],
                max_model_calls: 2,
                deadline_seconds: 600,
                ..Default::default()
            });
        if mode == "profiles" {
            selected.instructions = "Profile fixture revision 2".into();
            selected.skills[0].revision = "v2".into();
        }
        Some(selected)
    } else {
        None
    }
}

fn persist_fixture_turn_request(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    run_id: &str,
    model: &str,
) -> Result<(), CleanStoreError> {
    let selected = c
        .runs
        .iter()
        .find(|r| r.id == run_id)
        .ok_or(CleanStoreError::CorruptState)?;
    let planner_profile = selected
        .planner_profile
        .as_ref()
        .map(crate::agent_profiles::frozen)
        .transpose()
        .map_err(|_| CleanStoreError::CorruptState)?;
    let worker_profile = selected
        .worker_profile
        .as_ref()
        .map(crate::agent_profiles::frozen)
        .transpose()
        .map_err(|_| CleanStoreError::CorruptState)?;
    store.save_project_run_request(run_id,&ProjectAgentRequest{
        worker_profile,
            profile: planner_profile,run_id:run_id.into(),project_id:c.project_id.clone(),model:model.into(),history:c.messages.iter().map(|m|format!("{}: {}",m.role,m.content)).collect::<Vec<_>>().join("\n"),repositories:vec![],context:serde_json::json!({"purpose":"research","fixture":"No live repository source access; source refresh is verified by native tests."}).to_string(),conversation_context:serde_json::Value::Null,max_model_calls:2,deadline_seconds:600})?;
    Ok(())
}
