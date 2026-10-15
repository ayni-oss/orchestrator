//! Repeatable synthetic planner evaluations through the production agent/tool boundary.
//! A configured worker mounts its model credential normally; no credential enters a report.
use crate::{
    project_agent,
    workflow_trace::{ToolObservation, WorkflowHook},
};
use ayni_orchestrator_runtime::{
    agent_profile::{FrozenProfile, Role, Settings, SkillRevision},
    project_agent::{ProjectAgentRequest, ProjectAgentResult, ResearchRepository, research_key},
};
use rig_core::{client::completion::CompletionClient, completion::CompletionModel};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io, path::Path, time::Instant};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    id: String,
    input: String,
    expected_tools: Vec<String>,
    forbidden_tools: Vec<String>,
    contains: Vec<String>,
    excludes: Vec<String>,
    proposal_count: usize,
    skill: bool,
}
#[derive(Serialize)]
pub(crate) struct CaseReport {
    scenario: Scenario,
    request: ProjectAgentRequest,
    status: &'static str,
    failures: Vec<String>,
    latency_ms: u64,
    tools: Vec<ToolObservation>,
    result: Option<ProjectAgentResult>,
    model_usage: Vec<ayni_orchestrator_domain::ModelUsageReport>,
}

pub(crate) fn scenarios() -> io::Result<Vec<Scenario>> {
    serde_json::from_str(include_str!("../../../../evals/scenarios.json")).map_err(io::Error::other)
}
fn request(case: &Scenario, model: &str) -> io::Result<ProjectAgentRequest> {
    let mut settings = Settings::defaults(&Role::Planner);
    settings.max_model_calls = 8;
    settings.deadline_seconds = 90;
    if case.skill {
        settings.skills.push(SkillRevision {
            id: "readiness".into(), revision: "fixture-v1".into(),
            instructions: "Activate only for the phrase 'fixture readiness audit': call inspect_project_context, then include READINESS_CHECKED. For any other request do not activate or emit that marker.".into(),
        });
    }
    let freeze = |role: Role, settings: Settings| {
        FrozenProfile::freeze(
            role,
            "evaluation".into(),
            "synthetic-binding-v1".into(),
            model.into(),
            settings,
            vec!["repo-fixture".into()],
        )
    };
    Ok(ProjectAgentRequest {
        profile: Some(freeze(Role::Planner, settings)?),
        worker_profile: Some(freeze(Role::Worker, Settings::defaults(&Role::Worker))?),
        project_id: "project-evaluation".into(), run_id: format!("eval-{}", case.id),
        model: model.into(), history: case.input.clone(), conversation_context: Value::Null,
        context: json!({"repositories":[{"id":"repo-fixture","revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","runtime_profile_id":"runtime-fixture","runtime_ready":true,"coding_connection_id":"evaluation","coding_model":model}],"tasks":[{"id":"task-fixture","execution":"succeeded","quality":"failed","publication":"not_attempted","pull_request":null}]}).to_string(),
        repositories: vec![ResearchRepository {id:"repo-fixture".into(), repository_url:"https://github.com/ayni-oss/sandbox.git".into(), revision:"a".repeat(40), coding_connection_id:"evaluation".into(), coding_model:model.into(), runtime_profile_id:"runtime-fixture".into()}],
        max_model_calls:8, deadline_seconds:90,
    })
}
fn grade(case: &Scenario, tools: &[ToolObservation], result: &ProjectAgentResult) -> Vec<String> {
    let mut failures = tools
        .iter()
        .filter(|t| !t.succeeded)
        .map(|t| format!("tool_failed:{}", t.tool))
        .collect::<Vec<_>>();
    for name in &case.expected_tools {
        if !tools.iter().any(|t| &t.tool == name && t.succeeded) {
            failures.push(format!("missing_successful_tool:{name}"));
        }
    }
    for name in &case.forbidden_tools {
        if tools.iter().any(|t| &t.tool == name) {
            failures.push(format!("unexpected_tool:{name}"));
        }
    }
    for text in &case.contains {
        if !result.message.contains(text) {
            failures.push(format!("missing_output:{text}"));
        }
    }
    for text in &case.excludes {
        if result.message.contains(text) {
            failures.push(format!("unexpected_output:{text}"));
        }
    }
    if result.proposals.len() != case.proposal_count {
        failures.push("proposal_count".into());
    }
    for proposal in &result.proposals {
        if proposal.request.trim().is_empty()
            || proposal.findings.trim().is_empty()
            || proposal.scope.trim().is_empty()
            || proposal.dependency_environment_impact.trim().is_empty()
            || proposal.allowed_paths != ["README.md"]
            || proposal.acceptance_criteria.is_empty()
        {
            failures.push("incomplete_or_overbroad_plan".into());
        }
    }
    failures
}

pub(crate) async fn evaluate<M: CompletionModel + 'static>(
    case: Scenario,
    model: &str,
    transport: M,
) -> io::Result<CaseReport> {
    let request = request(&case, model)?;
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join(research_key("repo-fixture")))?;
    fs::write(
        root.path()
            .join(research_key("repo-fixture"))
            .join("README.md"),
        "# Demo\n",
    )?;
    let hook = WorkflowHook::new(project_agent::workflow(&request));
    let usage_root = tempfile::tempdir()?;
    let tracker = crate::coding::model_usage::UsageTracker::new(
        usage_root.path(),
        ayni_orchestrator_domain::ModelUsageStage::Coding,
        "openai",
        model,
    )?;
    let transport = crate::coding::model_usage::TrackedModel::new(transport, tracker.clone());
    let start = Instant::now();
    let result =
        project_agent::run_with_model(request.clone(), root.path(), transport, hook.clone()).await;
    tracker.fail_pending(None)?;
    let model_usage = fs::read_dir(usage_root.path())?
        .map(|entry| {
            let bytes =
                ayni_orchestrator_runtime::runtime_execution::read_bounded(&entry?.path(), 16384)?;
            serde_json::from_slice(&bytes).map_err(io::Error::other)
        })
        .collect::<io::Result<Vec<ayni_orchestrator_domain::ModelUsageReport>>>()?;
    let tools = hook.observations();
    let (status, failures, result) = result.map_or_else(
        |_| {
            if tools.is_empty() {
                (
                    "unavailable",
                    vec!["provider_or_agent_unavailable".into()],
                    None,
                )
            } else {
                ("failed", vec!["agent_tool_or_budget_failed".into()], None)
            }
        },
        |result| {
            let failures = grade(&case, &tools, &result);
            (
                if failures.is_empty() {
                    "passed"
                } else {
                    "failed"
                },
                failures,
                Some(result),
            )
        },
    );
    Ok(CaseReport {
        scenario: case,
        request,
        status,
        failures,
        latency_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        tools,
        result,
        model_usage,
    })
}

/// Runs bounded synthetic cases using the worker's existing mounted model credential.
/// # Errors
/// Only report serialization and fixture I/O errors escape; provider failures use safe codes.
pub async fn command(model: &str) -> io::Result<()> {
    let client = ayni_orchestrator_runtime::runtime_execution::read_bounded(
        Path::new("/model/api-key"),
        16384,
    )
    .ok()
    .and_then(|key| String::from_utf8(key).ok())
    .and_then(|key| ayni_orchestrator_providers::openai_client("openai", key.trim()).ok());
    let mut reports = Vec::new();
    if let Some(client) = client {
        for case in scenarios()? {
            reports.push(evaluate(case, model, client.completion_model(model)).await?);
        }
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({"schema_version":1,"mode":"model","model":model,"status":if reports.is_empty() {"unavailable"} else {"completed"},"cases":reports})
        )?
    );
    Ok(())
}

#[cfg(test)]
#[path = "evaluation_tests.rs"]
mod tests;
