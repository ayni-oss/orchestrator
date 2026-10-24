use super::{Worker, ledger::Ledger};
use ayni_orchestrator_events::task_execution::{TaskCommand, TaskOutcome, WorkerRequest, digest};
use ayni_orchestrator_providers::{
    coding,
    responses::{ResponseEvidence, ResponsesClient},
};
use ayni_orchestrator_scripted_executor::task_files::{self, FileChange};
use serde_json::json;
use std::{env, fs, time::Duration};
fn failure(reason: &str) -> TaskOutcome {
    TaskOutcome {
        state: if reason == "provider_uncertain" {
            "uncertain"
        } else {
            "failed"
        }
        .into(),
        safe_reason: reason.into(),
        ..Default::default()
    }
}
pub async fn execute(worker: &Worker, command: &TaskCommand, ledger: &mut Ledger) -> TaskOutcome {
    match execute_inner(worker, command, ledger).await {
        Ok(v) => v,
        Err(reason) => failure(reason),
    }
}
async fn execute_inner(
    worker: &Worker,
    c: &TaskCommand,
    ledger: &mut Ledger,
) -> std::result::Result<TaskOutcome, &'static str> {
    let state = ledger.state().map_err(|_| "execution_interrupted")?;
    if state == "sending" {
        return Err("provider_uncertain");
    }
    if c.provider == "scripted" {
        return scripted(worker, c).await;
    }
    if !matches!(c.provider.as_str(), "openai" | "anthropic")
        || env::var(if c.provider == "anthropic" {
            "ORCHESTRATOR_ANTHROPIC_ENABLED"
        } else {
            "ORCHESTRATOR_OPENAI_ENABLED"
        })
        .as_deref()
            != Ok("true")
    {
        return Err("provider_disabled");
    }
    // Environment lifecycle doctor requires an external engine. Actual Ayni
    // runtime verification happens only in the credential-free quality stages.
    let evidence = if let Some(e) = ledger.response().map_err(|_| "execution_interrupted")? {
        e
    } else {
        call(worker, c, ledger).await?
    };
    if evidence.model != c.model {
        return Err("provider_output_invalid");
    }
    if evidence
        .usage
        .total_tokens
        .is_some_and(|v| v > u64::from(c.max_tokens))
    {
        return Err("execution_limit");
    }
    if evidence.failure_reason.is_empty()
        && evidence.tool_calls.len() == 1
        && evidence.tool_calls[0].name == "request_task_recovery"
    {
        let advice: ayni_orchestrator_events::task_execution::TaskRecoveryAdvice =
            serde_json::from_str(&evidence.tool_calls[0].arguments)
                .map_err(|_| "provider_output_invalid")?;
        if !advice.validate() {
            return Err("provider_output_invalid");
        }
        return Ok(TaskOutcome {
            state: "failed".into(),
            safe_reason: "worker_question".into(),
            request_digest: ledger
                .request_digest()
                .map_err(|_| "execution_interrupted")?,
            response_id: evidence.response_id,
            input_tokens: evidence.usage.input_tokens,
            output_tokens: evidence.usage.output_tokens,
            total_tokens: evidence.usage.total_tokens,
            recovery_advice: Some(advice),
            ..Default::default()
        });
    }
    let changes: Vec<FileChange> = serde_json::from_value(coding::changes(&evidence)?)
        .map_err(|_| "provider_output_invalid")?;
    let patch = task_files::apply_scoped(&worker.source, &changes, &c.source_paths)
        .map_err(|_| "provider_output_invalid")?;
    Ok(TaskOutcome {
        state: "completed".into(),
        patch,
        request_digest: ledger
            .request_digest()
            .map_err(|_| "execution_interrupted")?,
        response_id: evidence.response_id,
        input_tokens: evidence.usage.input_tokens,
        output_tokens: evidence.usage.output_tokens,
        total_tokens: evidence.usage.total_tokens,
        safe_reason: String::new(),
        recovery_advice: None,
    })
}
async fn scripted(
    worker: &Worker,
    c: &TaskCommand,
) -> std::result::Result<TaskOutcome, &'static str> {
    if env::var("ORCHESTRATOR_TASK_FIXTURE_ENABLED").as_deref() != Ok("true") {
        return Err("provider_disabled");
    }
    // Only explicit deterministic acceptance uses this branch; it claims no Ayni compatibility.
    if c.objective == "Task self-development fixture" {
        if c.source_paths.len() != 1 {
            return Err("source_invalid");
        }
        let mut files =
            task_files::selected_context(&worker.source, &c.source_commit, &c.source_paths)
                .map_err(|_| "source_context_limit")?;
        let file = files.first_mut().ok_or("source_invalid")?;
        file.content
            .push_str("\n<!-- Verified through isolated Orchestrator task execution. -->\n");
        let patch = task_files::apply_scoped(&worker.source, &files, &c.source_paths)
            .map_err(|_| "source_invalid")?;
        return Ok(TaskOutcome {
            state: "completed".into(),
            patch,
            ..Default::default()
        });
    }
    if c.objective == "Task publication no-op fixture" {
        return Ok(TaskOutcome {
            state: "completed".into(),
            ..Default::default()
        });
    }
    let patch = task_files::apply_scoped(
        &worker.source,
        &[match &c.recovery {
            Some(recovery) => {
                if c.objective == "Task quality failure fixture"
                    && recovery.action == "repair_quality"
                {
                    FileChange {
                        path: "src/lib.rs".into(),
                        content: "#[test]\nfn repaired_quality() { assert_eq!(1, 1); }\n".into(),
                    }
                } else {
                    FileChange {
                        path: "task-recovery-result.txt".into(),
                        content: format!(
                            "Continued from {} with saved patch {}.\n",
                            recovery.parent_attempt_id, recovery.seed_patch_digest
                        ),
                    }
                }
            }
            None if matches!(
                c.objective.as_str(),
                "Task quality failure fixture" | "Task recovery fault fixture"
            ) =>
            {
                FileChange {
                    path: "src/lib.rs".into(),
                    content: "#[test]\nfn fails_quality() { assert_eq!(1, 2); }\n".into(),
                }
            }
            None => FileChange {
                path: "task-result.txt".into(),
                content: "Task execution fixture completed.\n".into(),
            },
        }],
        &c.source_paths,
    )
    .map_err(|_| "source_invalid")?;
    if c.objective == "Task cancellation fixture"
        || (c.objective == "Task recovery fault fixture" && c.recovery.is_some())
    {
        tokio::time::sleep(Duration::from_secs(120)).await;
    }
    Ok(TaskOutcome {
        state: "completed".into(),
        patch,
        ..Default::default()
    })
}

async fn call(
    worker: &Worker,
    c: &TaskCommand,
    ledger: &mut Ledger,
) -> std::result::Result<ResponseEvidence, &'static str> {
    let files = task_files::selected_context(&worker.source, &c.source_commit, &c.source_paths)
        .map_err(|_| "source_context_limit")?;
    let task = json!({"objective":c.objective,"scope":c.scope,"out_of_scope":c.out_of_scope,"acceptance_criteria":c.acceptance_criteria,"execution_limits":c.execution_limits,"source_commit":c.source_commit,"source_paths":c.source_paths,"attempt_id":c.attempt_id,"recovery":c.recovery.as_ref().map(|r|json!({"parent_attempt_id":r.parent_attempt_id,"action":r.action,"explanation":r.explanation,"quality_reason":r.quality_reason,"quality_evidence":r.quality_artifact,"answers":r.answers})),"files":files});
    let request = coding::prepare(
        &c.model,
        &c.instructions,
        &task,
        c.max_input_tokens,
        c.max_output_tokens,
        u64::from(c.max_tokens),
    )
    .map_err(|_| "source_context_limit")?;
    let key = fs::read_to_string(
        env::var(if c.provider == "anthropic" {
            "ORCHESTRATOR_ANTHROPIC_KEY_PATH"
        } else {
            "ORCHESTRATOR_OPENAI_KEY_PATH"
        })
        .map_err(|_| "provider_disabled")?,
    )
    .map_err(|_| "provider_disabled")?;
    let endpoint = if c.provider == "anthropic" {
        env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com/v1".into())
    } else {
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into())
    };
    let client = ResponsesClient::for_provider(&c.provider, key.trim(), &endpoint)
        .map_err(|_| "provider_disabled")?;
    ledger
        .sending(&request.bytes)
        .map_err(|_| "provider_uncertain")?;
    let permit = worker
        .call(&WorkerRequest::Sending {
            request_digest: digest(&request.bytes),
        })
        .await
        .map_err(|_| "provider_uncertain")?;
    if !permit.authorized {
        return Err("provider_uncertain");
    }
    let evidence = tokio::time::timeout(
        Duration::from_secs(u64::from(c.max_seconds)),
        client.send(&c.attempt_id, &request.bytes),
    )
    .await
    .map_err(|_| "provider_uncertain")?
    .map_err(|_| "provider_uncertain")?;
    ledger
        .retain_response(&evidence)
        .map_err(|_| "provider_uncertain")?;
    Ok(evidence)
}
