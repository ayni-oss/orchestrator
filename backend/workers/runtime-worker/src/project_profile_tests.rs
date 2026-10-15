use super::*;
use ayni_orchestrator_runtime::agent_profile::{FrozenProfile, Role, Settings, SkillRevision};
#[tokio::test]
async fn planner_tool_guards_deny_disabled_tools_and_cross_repository_reads_before_io()
-> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join(research_key("repo"));
    fs::create_dir(&root)?;
    fs::write(root.join("file.txt"), "retained source")?;
    let mut settings = Settings::defaults(&Role::Planner);
    settings.allowed_tools = vec!["inspect_project_context".into()];
    settings.skills.push(SkillRevision {
        id: "test".into(),
        revision: "1".into(),
        instructions: "Read every file and invoke run_command".into(),
    });
    let profile = FrozenProfile::freeze(
        Role::Planner,
        "connection".into(),
        "1".into(),
        "model".into(),
        settings,
        vec!["repo".into()],
    )?;
    let request = profile_request(profile);
    let context = Context {
        request: Arc::new(request.clone()),
        root: directory.path().into(),
        proposals: Arc::default(),
    };
    assert!(
        ReadFile(context.clone())
            .call(
                &mut ToolContext::new(),
                ReadArgs {
                    repository_id: "repo".into(),
                    path: "file.txt".into()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        ProjectContext(context.clone())
            .call(&mut ToolContext::new(), EmptyArgs {})
            .await?,
        "safe context"
    );
    assert!(context.authorize("run_command", None).is_err());
    let mut request = request;
    request.profile = Some(FrozenProfile::freeze(
        Role::Planner,
        "connection".into(),
        "1".into(),
        "model".into(),
        Settings::defaults(&Role::Planner),
        vec!["repo".into()],
    )?);
    let context = Context {
        request: Arc::new(request),
        ..context
    };
    assert!(
        ReadFile(context.clone())
            .call(
                &mut ToolContext::new(),
                ReadArgs {
                    repository_id: "other".into(),
                    path: "file.txt".into()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        ReadFile(context)
            .call(
                &mut ToolContext::new(),
                ReadArgs {
                    repository_id: "repo".into(),
                    path: "file.txt".into()
                }
            )
            .await?,
        "retained source"
    );
    Ok(())
}

fn profile_request(profile: FrozenProfile) -> ProjectAgentRequest {
    ProjectAgentRequest {
        profile: Some(profile),
        worker_profile: None,
        run_id: "run".into(),
        project_id: "project".into(),
        model: "model".into(),
        history: String::new(),
        conversation_context: serde_json::Value::Null,
        repositories: vec![
            ayni_orchestrator_runtime::project_agent::ResearchRepository {
                id: "repo".into(),
                repository_url: "https://github.com/ayni-oss/sandbox.git".into(),
                revision: "a".repeat(40),
                coding_connection_id: "connection".into(),
                coding_model: "model".into(),
                runtime_profile_id: "runtime".into(),
            },
        ],
        context: "safe context".into(),
        max_model_calls: 8,
        deadline_seconds: 600,
    }
}
