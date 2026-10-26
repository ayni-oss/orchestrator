//! Observation failures never authorize another execution or paid provider call.
use super::{
    Duration, ExecutionResult, ProviderRuntime, RuntimeResult, authority, bounded_json,
    decode_command, digest, io, json, validate_result,
};
use ayni_orchestrator_events::progress::ExecutionProgress;

pub(super) async fn forward(
    http: &reqwest::Client,
    worker_url: &str,
    token: &str,
    command_bytes: &[u8],
    runtime: &ProviderRuntime,
    after: u64,
) -> RuntimeResult<u64> {
    let (identity, _, _) = decode_command(command_bytes)?;
    let response = http
        .post(format!("{worker_url}/v1/provider-progress"))
        .bearer_auth(token)
        .json(&crate::session_worker::progress::ProgressInput {
            command_id: identity.id,
            command_digest: digest(command_bytes),
            after_sequence: after,
        })
        .send()
        .await?
        .error_for_status()?;
    let observations: Vec<ExecutionProgress> = bounded_json(response, 128 * 1024).await?;
    if observations.len() > 65 {
        return Err(io::Error::other("progress bound exceeded").into());
    }
    let mut sequence = after;
    for progress in observations {
        if !progress.valid() || progress.sequence <= sequence {
            return Err(io::Error::other("progress order invalid").into());
        }
        let next = progress.sequence;
        authority(
            http,
            json!({"command_bytes":command_bytes,"action":"progress","progress":progress}),
            runtime,
        )
        .await?;
        sequence = next;
    }
    Ok(sequence)
}

pub(super) async fn execute_fixture(
    http: reqwest::Client,
    address: &str,
    token: &str,
    command_bytes: &[u8],
) -> RuntimeResult<ExecutionResult> {
    let worker_url = format!("http://{address}:8081");
    let runtime = ProviderRuntime::from_environment("scripted").ok();
    let send = http
        .post(format!("{worker_url}/v1/execute"))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(command_bytes.to_vec())
        .send();
    tokio::pin!(send);
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut sequence = 0;
    let response = loop {
        tokio::select! {
            response = &mut send => break response?.error_for_status()?,
            _ = tick.tick() => if let Some(runtime) = &runtime
                && let Ok(next) = forward(&http, &worker_url, token, command_bytes, runtime, sequence).await { sequence = next; },
        }
    };
    if let Some(runtime) = &runtime {
        let _ = forward(&http, &worker_url, token, command_bytes, runtime, sequence).await;
    }
    let (identity, _, _) = decode_command(command_bytes)?;
    let result: ExecutionResult = bounded_json(response, 32768).await?;
    validate_result(result, &identity.id, &digest(command_bytes))
}
