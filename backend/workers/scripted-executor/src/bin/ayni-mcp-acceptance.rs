//! Deterministic driver for an owned deployed session-worker/MCP fixture Pod.
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        AgentExecutionConfiguration, ConversationContextManifest, ConversationContextRecord,
        ConversationTurnBinding, ConversationTurnRequested, SessionExecutionScope,
    },
    workflow::{APPLICATION_SOURCE, WorkflowEventMetadata, encode_command},
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::error::Error;
#[path = "mcp_acceptance/external_effects.rs"]
mod external_effects;
const TOKEN: &str = "public-mcp-acceptance-session-token-01234567890123456789";
fn id(n: u8) -> String {
    format!("019a0000-0000-7000-8000-{n:012}")
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
type DriverResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
#[tokio::main]
async fn main() -> DriverResult<()> {
    let worker = fixture_endpoint("MCP_WORKER_URL")?;
    let fixture = fixture_endpoint("MCP_FIXTURE_URL")?;
    let replay = std::env::args().any(|arg| arg == "--replay");
    let anthropic = std::env::args().any(|arg| arg == "--anthropic");
    let provider = if anthropic { "anthropic" } else { "openai" };
    let client = reqwest::Client::new();
    let before = read_stats(&client, &fixture).await?;
    let (prepared, command_digest) =
        prepare_fixture(&client, &worker, provider, "fixture:mcp", 6).await?;
    let result = if replay {
        require_result(&prepared)?;
        prepared["result"].clone()
    } else {
        execute_success(&client, &worker, provider, &prepared).await?
    };
    let after = read_stats(&client, &fixture).await?;
    validate_effects(&before, &after, replay)?;
    let external_effects =
        external_effects::profiles(&client, &worker, &fixture, provider, replay).await?;
    let provider_failures = if anthropic {
        failure_profiles(&client, &worker, &fixture, replay).await?
    } else {
        Vec::new()
    };
    println!(
        "{}",
        json!({"outcome":"passed","profile":"deployed-provider-mcp-worker","provider":provider,"replay":replay,"checks":["selected-provider-headers","tool-discovery","frozen-tools","exact-external-effect-consent","retained-tool-output","pod-replacement-replay"],"provider_failure_checks":anthropic,"external_effects":after["mcpEffects"],"command_id":id(6),"command_digest":command_digest,"success_result":result,"external_effect_failures":external_effects,"provider_failures":provider_failures,"final_stats":read_stats(&client, &fixture).await?})
    );
    Ok(())
}
fn fixture_endpoint(name: &str) -> DriverResult<String> {
    let endpoint = std::env::var(name)?;
    let url = reqwest::Url::parse(&endpoint)?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
        return Err("fixture requires loopback port-forwards".into());
    }
    Ok(endpoint)
}
async fn read_stats(client: &reqwest::Client, fixture: &str) -> DriverResult<Value> {
    Ok(client
        .get(format!("{fixture}/stats"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
fn require_result(prepared: &Value) -> DriverResult<()> {
    if prepared["result"].is_null() {
        return Err("retained terminal fixture result missing".into());
    }
    Ok(())
}
fn validate_effects(before: &Value, after: &Value, replay: bool) -> DriverResult<()> {
    if after["mcpEffects"] != 1 {
        return Err("external effect count must equal one".into());
    }
    if replay && (before["calls"] != after["calls"] || before["mcpReads"] != after["mcpReads"]) {
        return Err("replay contacted provider or MCP".into());
    }
    Ok(())
}
fn send_request(
    client: &reqwest::Client,
    worker: &str,
    prepared: &Value,
    seed: u8,
) -> reqwest::RequestBuilder {
    client.post(format!("{worker}/v1/provider-send")).bearer_auth(TOKEN).json(&json!({"command_id":id(seed),"request_digest":prepared["request_digest"],"allowed":true,"state":"sending"}))
}
async fn execute_success(
    client: &reqwest::Client,
    worker: &str,
    provider: &str,
    prepared: &Value,
) -> DriverResult<Value> {
    let result: Value = send_request(client, worker, prepared, 6)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !result["failure_reason"].as_str().unwrap_or("").is_empty() {
        return Err(format!("fixture failed: {result}").into());
    }
    if result["provider_call"]["provider"] != provider
        || result["provider_call"]["totalTokens"] != "40"
    {
        return Err("selected provider or cumulative usage evidence is incorrect".into());
    }
    let tools = result["provider_call"]["toolInvocations"]
        .as_array()
        .ok_or("tool evidence missing")?;
    if tools.len() != 2 || tools.iter().any(|tool| tool["state"] != "completed") {
        return Err("scoped MCP evidence incomplete".into());
    }
    Ok(result)
}

type FixtureCommand = (Vec<u8>, Vec<ConversationContextRecord>);
fn fixture_command(
    provider: &str,
    mode: &str,
    seed: u8,
) -> Result<FixtureCommand, Box<dyn Error + Send + Sync>> {
    let context = vec![ConversationContextRecord {
        message_id: id(seed + 2),
        sequence: 1,
        role: "user".into(),
        content: mode.into(),
    }];
    let mut hash = Sha256::new();
    hash.update(b"ayni-conversation-context-v1");
    for value in [
        context[0].message_id.as_bytes(),
        &1_u64.to_be_bytes(),
        b"user",
        mode.as_bytes(),
    ] {
        hash.update(u64::try_from(value.len())?.to_be_bytes());
        hash.update(value);
    }
    let command = ConversationTurnRequested {
        binding: Some(ConversationTurnBinding {
            workspace_id: id(1),
            project_id: id(2),
            conversation_id: id(3),
            turn_id: id(seed + 1),
            assignment_id: id(seed + 3),
            executor_generation: 1,
            execution_binding_revision: if provider == "anthropic" {
                "anthropic-messages-v1"
            } else {
                "openai-responses-v1"
            }
            .into(),
            context_revision: 1,
            command_event_id: id(seed),
            command_digest: String::new(),
        }),
        input_message_id: id(seed + 2),
        input_content: mode.into(),
        context: context.clone(),
        frozen_configuration: Some(AgentExecutionConfiguration {
            revision_id: id(5),
            instructions: "Only configured fixture tools.".into(),
            provider: provider.into(),
            model: "fixture-model".into(),
            max_input_tokens: 65_536,
            max_output_tokens: 4096,
            max_total_tokens: 131_072,
            idle_timeout_seconds: 3600,
        }),
        context_manifest: Some(ConversationContextManifest {
            id: id(seed + 1),
            digest: format!("sha256:{:x}", hash.finalize()),
            record_count: 1,
            inline_complete: true,
        }),
        session: Some(SessionExecutionScope {
            session_id: id(4),
            generation: 1,
            idle_timeout_seconds: 3600,
        }),
        project_status: None,
    };
    let bytes = encode_command(
        &EventIdentity {
            source: APPLICATION_SOURCE.into(),
            id: id(seed),
        },
        &WorkflowEventMetadata {
            produced_at: "2026-10-24T12:00:00Z".into(),
            traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
        },
        &command,
    )?;
    // Production replays retained bytes. Re-serializing a CloudEvent can reorder extensions.
    let directory = std::path::PathBuf::from(std::env::var("MCP_COMMAND_CACHE")?);
    let path = directory.join(format!("{provider}-{seed}.json"));
    let bytes = if path.exists() {
        std::fs::read(&path)?
    } else {
        std::fs::write(&path, &bytes)?;
        bytes
    };
    Ok((bytes, context))
}

async fn prepare_fixture(
    client: &reqwest::Client,
    worker: &str,
    provider: &str,
    mode: &str,
    seed: u8,
) -> DriverResult<(Value, String)> {
    let (bytes, context) = fixture_command(provider, mode, seed)?;
    let command_digest = digest(&bytes);
    let prepared = client
        .post(format!("{worker}/v1/provider-prepare"))
        .bearer_auth(TOKEN)
        .json(&json!({"command_bytes":bytes,"context":context}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok((prepared, command_digest))
}
async fn failure_profiles(
    client: &reqwest::Client,
    worker: &str,
    fixture: &str,
    replay: bool,
) -> DriverResult<Vec<Value>> {
    let mut results = Vec::new();
    for (mode, seed) in [("fixture:disconnect", 20), ("fixture:delay", 40)] {
        let before = read_stats(client, fixture).await?;
        let (prepared, command_digest) =
            prepare_fixture(client, worker, "anthropic", mode, seed).await?;
        let result = if replay {
            require_result(&prepared)?;
            prepared["result"].clone()
        } else {
            execute_failure(
                client,
                worker,
                fixture,
                &before,
                &prepared,
                &command_digest,
                seed,
            )
            .await?
        };
        validate_failure(&result, seed)?;
        results.push(result);
        let after = read_stats(client, fixture).await?;
        let (retained, _) = prepare_fixture(client, worker, "anthropic", mode, seed).await?;
        require_result(&retained)?;
        let final_stats = read_stats(client, fixture).await?;
        if after["calls"] != final_stats["calls"] || (replay && before["calls"] != after["calls"]) {
            return Err("uncertainty or cancellation replay resent provider request".into());
        }
    }
    Ok(results)
}
async fn execute_failure(
    client: &reqwest::Client,
    worker: &str,
    fixture: &str,
    before: &Value,
    prepared: &Value,
    command_digest: &str,
    seed: u8,
) -> DriverResult<Value> {
    let request = send_request(client, worker, prepared, seed);
    let result = if seed == 40 {
        cancel_in_flight(client, worker, fixture, before, request, command_digest).await?
    } else {
        request.send().await?.error_for_status()?.json().await?
    };
    validate_failure(&result, seed)?;
    Ok(result)
}
fn validate_failure(result: &Value, seed: u8) -> DriverResult<()> {
    if seed == 20 && result["provider_call"]["state"] != "uncertain" {
        return Err("disconnect did not retain uncertainty".into());
    }
    if seed == 40
        && !matches!(
            result["provider_call"]["state"].as_str(),
            Some("uncertain" | "cancelled")
        )
    {
        return Err("cancellation result incorrect".into());
    }
    if !result["response_content"].as_str().unwrap_or("").is_empty() {
        return Err("failed/cancelled provider output escaped".into());
    }
    Ok(())
}
async fn cancel_in_flight(
    client: &reqwest::Client,
    worker: &str,
    fixture: &str,
    before: &Value,
    request: reqwest::RequestBuilder,
    command_digest: &str,
) -> DriverResult<Value> {
    let sending = tokio::spawn(async move {
        request
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await
    });
    wait_for_delay(client, fixture, before).await?;
    client
        .post(format!("{worker}/v1/cancel"))
        .bearer_auth(TOKEN)
        .json(&json!({"command_id":id(40),"command_digest":command_digest}))
        .send()
        .await?
        .error_for_status()?;
    Ok(sending.await??)
}
async fn wait_for_delay(
    client: &reqwest::Client,
    fixture: &str,
    before: &Value,
) -> DriverResult<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let stats = read_stats(client, fixture).await?;
        if stats["anthropicModes"]["delay"].as_u64().unwrap_or(0)
            > before["anthropicModes"]["delay"].as_u64().unwrap_or(0)
        {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err("delayed provider request was not observed".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
