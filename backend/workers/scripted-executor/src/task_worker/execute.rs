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
        if env::var("ORCHESTRATOR_TASK_FIXTURE_ENABLED").as_deref() != Ok("true") {
            return Err("provider_disabled");
        }
        // Only explicit deterministic acceptance uses this branch; it claims no Ayni compatibility.
        if c.objective == "Task publication no-op fixture" {
            return Ok(TaskOutcome {
                state: "completed".into(),
                ..Default::default()
            });
        }
        let patch = task_files::apply(
            &worker.source,
            &[if c.objective == "Task quality failure fixture" {
                FileChange {
                    path: "src/lib.rs".into(),
                    content: "#[test]\nfn fails_quality() { assert_eq!(1, 2); }\n".into(),
                }
            } else {
                FileChange {
                    path: "task-result.txt".into(),
                    content: "Task execution fixture completed.\n".into(),
                }
            }],
        )
        .map_err(|_| "source_invalid")?;
        if c.objective == "Task cancellation fixture" {
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
        return Ok(TaskOutcome {
            state: "completed".into(),
            patch,
            ..Default::default()
        });
    }
    if c.provider != "openai" || env::var("ORCHESTRATOR_OPENAI_ENABLED").as_deref() != Ok("true") {
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
    let changes: Vec<FileChange> = serde_json::from_value(coding::changes(&evidence)?)
        .map_err(|_| "provider_output_invalid")?;
    let patch =
        task_files::apply(&worker.source, &changes).map_err(|_| "provider_output_invalid")?;
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
    })
}
async fn call(
    worker: &Worker,
    c: &TaskCommand,
    ledger: &mut Ledger,
) -> std::result::Result<ResponseEvidence, &'static str> {
    let files = task_files::context(&worker.source, &c.source_commit)
        .map_err(|_| "source_context_limit")?;
    let task = json!({"objective":c.objective,"scope":c.scope,"out_of_scope":c.out_of_scope,"acceptance_criteria":c.acceptance_criteria,"execution_limits":c.execution_limits,"source_commit":c.source_commit,"files":files});
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
        env::var("ORCHESTRATOR_OPENAI_KEY_PATH").map_err(|_| "provider_disabled")?,
    )
    .map_err(|_| "provider_disabled")?;
    let endpoint =
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
    let client = ResponsesClient::new(key.trim(), &endpoint).map_err(|_| "provider_disabled")?;
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
