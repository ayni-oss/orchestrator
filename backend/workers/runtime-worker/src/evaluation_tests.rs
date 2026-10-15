use super::*;
use rig_core::{
    completion::{CompletionError, CompletionRequest, CompletionResponse, Usage},
    message::AssistantContent,
    streaming::StreamingCompletionResponse,
};
use std::{collections::VecDeque, sync::Mutex};

struct Scripted(Mutex<VecDeque<Vec<AssistantContent>>>);
impl CompletionModel for Scripted {
    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        assert!(!request.record_telemetry_content);
        assert!(
            request
                .tools
                .iter()
                .all(|t| !matches!(t.name.as_str(), "run_command" | "commit_changes"))
        );
        let choice = self
            .0
            .lock()
            .map_err(|_| CompletionError::ResponseError("fixture_lock".into()))?
            .pop_front()
            .ok_or_else(|| CompletionError::ResponseError("fixture_exhausted".into()))?;
        Ok(CompletionResponse::new(choice, Usage::default(), "fixture").with_raw(json!({"model":"scripted-v1","usage":{"input_tokens":16,"output_tokens":8,"total_tokens":24}})))
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        Err(CompletionError::ResponseError(
            "fixture_streaming_disabled".into(),
        ))
    }
}
fn tool(name: &str, args: Value) -> AssistantContent {
    AssistantContent::tool_call(format!("call-{name}"), name, args)
}
fn script(case: &Scenario) -> Scripted {
    let mut turns = Vec::new();
    match case.id.as_str() {
        "planning" => {
            turns.push(vec![tool(
                "read_repository_file",
                json!({"repository_id":"repo-fixture","path":"README.md"}),
            )]);
            turns.push(vec![tool("prepare_task_proposal", json!({"findings":"README title is Demo","scope":"README.md only","dependency_environment_impact":"none","repository_id":"repo-fixture","revision":"a".repeat(40),"runtime_profile_id":"runtime-fixture","coding_connection_id":"evaluation","coding_model":"scripted-v1","request":"Change title to Example","acceptance_criteria":["Title is Example"],"allowed_paths":["README.md"],"allow_test_edits":false,"forbidden_paths":[],"coding_turns":8,"deadline_seconds":600,"action":"create_task","target_task_id":""}))]);
        }
        "skill-activation" | "result-interpretation" => {
            turns.push(vec![tool("inspect_project_context", json!({}))]);
        }
        _ => {}
    }
    let text = match case.id.as_str() {
        "clarification" => "Which behavior should change?",
        "planning" => "Proposal prepared; review and approve it before execution.",
        "read-only" => "I cannot execute commands in the research role.",
        "skill-activation" => "READINESS_CHECKED",
        "skill-non-activation" => "HELLO",
        _ => "EXECUTION_SUCCEEDED QUALITY_FAILED PUBLICATION_NOT_ATTEMPTED. No PR exists.",
    };
    turns.push(vec![AssistantContent::text(text)]);
    Scripted(Mutex::new(turns.into()))
}

#[tokio::test]
async fn scripted_scenarios_exercise_real_agent_tools_and_retain_reproducible_reports()
-> io::Result<()> {
    let mut reports = Vec::new();
    for case in scenarios()? {
        let model = script(&case);
        let report = evaluate(case, "scripted-v1", model).await?;
        assert_eq!(
            report.status, "passed",
            "{}: {:?}",
            report.scenario.id, report.failures
        );
        let result = report
            .result
            .as_ref()
            .ok_or_else(|| io::Error::other("fixture_result"))?;
        assert!(result.model_calls > 0);
        assert_eq!(
            result.token_usage.total_tokens,
            Some(u64::from(result.model_calls) * 24)
        );
        report.request.validate_profile()?;
        reports.push(report);
    }
    if let Ok(path) = std::env::var("AYNI_EVAL_REPORT_PATH") {
        fs::write(
            path,
            serde_json::to_vec_pretty(
                &json!({"schema_version":1,"mode":"scripted","cases":reports}),
            )?,
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn missing_activation_and_provider_failure_cannot_pass_as_success() -> io::Result<()> {
    let case = scenarios()?
        .into_iter()
        .find(|s| s.id == "skill-activation")
        .ok_or_else(|| io::Error::other("case"))?;
    let no_tool = Scripted(Mutex::new(
        vec![vec![AssistantContent::text("READINESS_CHECKED")]].into(),
    ));
    let report = evaluate(case.clone(), "scripted-v1", no_tool).await?;
    assert_eq!(report.status, "failed");
    assert!(
        report
            .failures
            .contains(&"missing_successful_tool:inspect_project_context".into())
    );
    let report = evaluate(case, "scripted-v1", Scripted(Mutex::default())).await?;
    assert_eq!(report.status, "unavailable");
    assert!(report.result.is_none());
    Ok(())
}

#[test]
fn rubric_detects_unwanted_tool_and_skill_activation() -> io::Result<()> {
    let case = scenarios()?
        .into_iter()
        .find(|s| s.id == "skill-non-activation")
        .ok_or_else(|| io::Error::other("case"))?;
    let result = ProjectAgentResult {
        run_id: "test".into(),
        message: "HELLO READINESS_CHECKED".into(),
        proposals: vec![],
        model_calls: 1,
        token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
    };
    let failures = grade(
        &case,
        &[ToolObservation {
            tool: "inspect_project_context".into(),
            succeeded: true,
            latency_ms: 1,
        }],
        &result,
    );
    assert!(failures.iter().any(|s| s.starts_with("unexpected_tool:")));
    assert!(failures.iter().any(|s| s.starts_with("unexpected_output:")));
    Ok(())
}

#[tokio::test]
async fn invalid_mutation_tool_is_observed_and_cannot_pass_read_only_case() -> io::Result<()> {
    let case = scenarios()?
        .into_iter()
        .find(|s| s.id == "read-only")
        .ok_or_else(|| io::Error::other("case"))?;
    let attempted_write = Scripted(Mutex::new(
        vec![vec![tool("run_command", json!({"command":"echo denied"}))]].into(),
    ));
    let report = evaluate(case, "scripted-v1", attempted_write).await?;
    assert_ne!(report.status, "passed");
    assert!(
        report
            .tools
            .iter()
            .any(|t| t.tool == "run_command" && !t.succeeded)
    );
    assert_eq!(report.model_usage.len(), 1);
    Ok(())
}
