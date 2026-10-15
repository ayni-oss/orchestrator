use super::*;
use ayni_orchestrator_runtime::agent_profile::{FrozenProfile, Role, Settings};
use rig_agent::tool::Tool;
#[tokio::test]
async fn worker_tool_guards_reject_commands_before_process_execution() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let workspace = directory.path().join("workspace");
    let output = directory.path().join("output");
    fs::create_dir(&output)?;
    let mut spec = tests::fixture(&workspace)?;
    let mut settings = Settings::defaults(&Role::Worker);
    settings.allowed_tools = vec!["ayni_check".into()];
    spec.profile = Some(FrozenProfile::freeze(
        Role::Worker,
        "connection".into(),
        "1".into(),
        spec.model.clone(),
        settings,
        vec!["repo".into()],
    )?);
    let activity = activity::ActivityLog::new(Some(output.join("activity.json")));
    let context = sessions::Context {
        spec: Arc::new(spec),
        workspace: workspace.clone(),
        output,
        state: Arc::default(),
        serial: Arc::default(),
        activity,
    };
    let args = serde_json::from_value(serde_json::json!({"command":"touch forbidden.txt"}))
        .map_err(io::Error::other)?;
    assert!(
        tools::RunCommand(context)
            .call(&mut rig_agent::tool::ToolContext::new(), args)
            .await
            .is_err()
    );
    assert!(!workspace.join("forbidden.txt").exists());
    Ok(())
}
