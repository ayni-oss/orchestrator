//! The model can request reads; only frozen session inputs grant their authority.
use super::*;
use ayni_orchestrator_events::v1::ToolInvocationEvidence;
use ayni_orchestrator_providers::responses::{
    FunctionCall, ResponseEvidence, TokenUsage, prepare_items,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize)]
struct RetainedTool {
    call: FunctionCall,
    input_digest: String,
    output: String,
    state: String,
}

#[derive(Deserialize, Serialize)]
struct Exchange {
    call_id: String,
    request: Vec<u8>,
    response: Option<ResponseEvidence>,
}

/// Every network exchange commits `sending` before I/O and retains its complete
/// normalized result before a local read or another request can run.
#[derive(Deserialize, Serialize)]
pub(super) struct ProviderLoop {
    request: Value,
    phase: String,
    round: u8,
    response: Option<ResponseEvidence>,
    usage: TokenUsage,
    sources: BTreeMap<String, Value>,
    tools: BTreeMap<String, RetainedTool>,
    exchanges: Vec<Exchange>,
}

impl ProviderLoop {
    pub(super) fn new(input: &PrepareInput, bytes: &[u8]) -> Result<Self, StatusCode> {
        let (_, _, command) =
            decode_command(&input.command_bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
        let config = command
            .frozen_configuration
            .ok_or(StatusCode::BAD_REQUEST)?;
        let mut sources = BTreeMap::new();
        sources.insert(
            "read_workspace_instructions".into(),
            json!({"revision_id":config.revision_id,"instructions":config.instructions}),
        );
        sources.insert("read_conversation_history".into(),json!({"records":input.context.iter().map(|record|json!({"message_id":record.message_id,"sequence":record.sequence,"role":record.role,"content":record.content})).collect::<Vec<_>>()}));
        if let Some(snapshot) = command.project_status {
            sources.insert("read_project_status".into(),json!({"snapshot_id":snapshot.snapshot_id,"project_id":snapshot.project_id,"objective":snapshot.objective,"status":snapshot.status,"context_revision":snapshot.context_revision,"project_resource_version":snapshot.project_resource_version}));
        }
        Ok(Self {
            request: serde_json::from_slice(bytes).map_err(|_| StatusCode::BAD_REQUEST)?,
            phase: "ready".into(),
            round: 0,
            response: None,
            usage: TokenUsage {
                input_tokens: Some(0),
                output_tokens: Some(0),
                total_tokens: Some(0),
            },
            sources,
            tools: BTreeMap::new(),
            exchanges: Vec::new(),
        })
    }

    fn project(&self, evidence: &mut ProviderCallEvidence) {
        evidence.input_tokens = self.usage.input_tokens;
        evidence.output_tokens = self.usage.output_tokens;
        evidence.total_tokens = self.usage.total_tokens;
        if let Some(response) = &self.response {
            evidence.response_id.clone_from(&response.response_id);
            evidence.reported_model.clone_from(&response.model);
        }
        evidence.tool_invocations = self
            .tools
            .iter()
            .map(|(id, tool)| ToolInvocationEvidence {
                invocation_id: id.clone(),
                name: tool.call.name.clone(),
                input_digest: tool.input_digest.clone(),
                output_digest: digest(tool.output.as_bytes()),
                state: tool.state.clone(),
            })
            .collect();
    }
}

fn checkpoint(state: &WorkerState, command_id: &str, execution: &ProviderLoop) -> Result<(), ()> {
    let json = serde_json::to_string(execution).map_err(|_| ())?;
    let connection = state.connection.lock().map_err(|_| ())?;
    let retained = retained_bytes(&connection, command_id)?;
    // Preserve room for the bounded terminal result and its application handoff.
    if retained.saturating_add(i64::try_from(json.len() + 64 * 1024).map_err(|_| ())?)
        > MAX_RETAINED_BYTES
    {
        return Err(());
    }
    connection
        .execute(
            "UPDATE provider_loops SET state=?2 WHERE command_id=?1",
            params![command_id, json],
        )
        .map_err(|_| ())?;
    drop(connection);
    Ok(())
}

fn retained_bytes(connection: &Connection, excluded_command: &str) -> Result<i64, ()> {
    connection.query_row("SELECT (SELECT COALESCE(sum(COALESCE(length(request),0)+length(CAST(evidence AS BLOB))),0) FROM provider_calls)+(SELECT COALESCE(sum(length(CAST(result AS BLOB))),0) FROM results)+(SELECT COALESCE(sum(length(CAST(state AS BLOB))),0) FROM provider_loops WHERE command_id<>?1)",[excluded_command],|row|row.get(0)).map_err(|_|())
}

fn cancelled(state: &WorkerState, command_id: &str) -> bool {
    state
        .connection
        .lock()
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT cancelled FROM provider_calls WHERE command_id=?1",
                    [command_id],
                    |row| row.get::<_, bool>(0),
                )
                .ok()
        })
        .unwrap_or(true)
}

async fn paid_request(
    state: &WorkerState,
    call_id: &str,
    command_id: &str,
    bytes: &[u8],
) -> Result<ResponseEvidence, ()> {
    let client = state
        .provider
        .lock()
        .ok()
        .and_then(|client| client.clone())
        .ok_or(())?;
    let response = client.send(call_id, bytes);
    tokio::pin!(response);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result=&mut response => return result.map_err(|_|()),
            _=interval.tick()=>if cancelled(state,command_id) { return Err(()); },
        }
    }
}

fn add_usage(total: &mut TokenUsage, usage: &TokenUsage) {
    let sum = |old: Option<u64>, new: Option<u64>| {
        old.and_then(|old| new.and_then(|new| old.checked_add(new)))
    };
    total.input_tokens = sum(total.input_tokens, usage.input_tokens);
    total.output_tokens = sum(total.output_tokens, usage.output_tokens);
    total.total_tokens = sum(total.total_tokens, usage.total_tokens);
}

fn fail(evidence: &mut ProviderCallEvidence, reason: &str) -> (String, String) {
    evidence.state = "failed".into();
    (String::new(), reason.into())
}

pub(super) async fn execute(
    state: &WorkerState,
    input: &SendInput,
    request: &[u8],
    evidence: &mut ProviderCallEvidence,
) -> Result<(String, String), ()> {
    let json: Option<String> = state
        .connection
        .lock()
        .map_err(|_| ())?
        .query_row(
            "SELECT state FROM provider_loops WHERE command_id=?1",
            [&input.command_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| ())?;
    let Some(json) = json else {
        // A prepared call retained by the previous executable keeps its exact
        // request bytes and its one-request allowance; it cannot acquire tools.
        let response = paid_request(state, &input.command_id, &input.command_id, request).await?;
        evidence.response_id = response.response_id;
        evidence.reported_model = response.model;
        evidence.input_tokens = response.usage.input_tokens;
        evidence.output_tokens = response.usage.output_tokens;
        evidence.total_tokens = response.usage.total_tokens;
        let failure_reason = if response.tool_calls.is_empty() {
            response.failure_reason
        } else {
            "provider_tool_rejected".into()
        };
        evidence.state = if failure_reason.is_empty() {
            "completed"
        } else {
            "failed"
        }
        .into();
        return Ok((response.content, failure_reason));
    };
    let mut execution: ProviderLoop = serde_json::from_str(&json).map_err(|_| ())?;
    loop {
        execution.project(evidence);
        if let Some(outcome) = advance_exchange(state, input, &mut execution, evidence).await? {
            return Ok(outcome);
        }
        let response = execution.response.clone().ok_or(())?;
        if !response.failure_reason.is_empty() {
            return Ok(fail(evidence, &response.failure_reason));
        }
        if response.tool_calls.is_empty() {
            evidence.state = "completed".into();
            return Ok((response.content, String::new()));
        }
        if !input.allowed {
            return Ok(fail(evidence, "provider_interrupted"));
        }
        if cancelled(state, &input.command_id) {
            evidence.state = "cancelled".into();
            return Ok((String::new(), "cancelled".into()));
        }
        // Unknown usage retains the reservation and blocks another paid call.
        if execution.usage.input_tokens.is_none()
            || execution.usage.output_tokens.is_none()
            || execution.usage.total_tokens.is_none()
        {
            return Ok(fail(evidence, "provider_usage_unknown"));
        }
        let mut outputs = Vec::new();
        for call in &response.tool_calls {
            if cancelled(state, &input.command_id) {
                evidence.state = "cancelled".into();
                return Ok((String::new(), "cancelled".into()));
            }
            match invoke(&mut execution, call) {
                Ok(output) => outputs.push((call.clone(), output)),
                Err(reason) => {
                    checkpoint(state, &input.command_id, &execution)?;
                    execution.project(evidence);
                    return Ok(fail(evidence, reason));
                }
            }
            checkpoint(state, &input.command_id, &execution)?;
            execution.project(evidence);
        }
        if execution.round >= 8 {
            return Ok(fail(evidence, "provider_tool_limit"));
        }
        if !prepare_continuation(&mut execution, evidence, outputs)? {
            return Ok(fail(evidence, "provider_budget_exhausted"));
        }
        checkpoint(state, &input.command_id, &execution)?;
    }
}

async fn advance_exchange(
    state: &WorkerState,
    input: &SendInput,
    execution: &mut ProviderLoop,
    evidence: &mut ProviderCallEvidence,
) -> Result<Option<(String, String)>, ()> {
    match execution.phase.as_str() {
        "ready" => {
            if !input.allowed {
                if execution.round == 0 {
                    return Err(());
                }
                return Ok(Some(fail(evidence, "provider_interrupted")));
            }
            if cancelled(state, &input.command_id) {
                evidence.state = "cancelled".into();
                return Ok(Some((String::new(), "cancelled".into())));
            }
            execution.phase = "sending".into();
            let bytes = serde_json::to_vec(&execution.request).map_err(|_| ())?;
            let call_id = if execution.round == 0 {
                input.command_id.clone()
            } else {
                format!("{}-{}", input.command_id, execution.round)
            };
            execution.exchanges.push(Exchange {
                call_id: call_id.clone(),
                request: bytes.clone(),
                response: None,
            });
            checkpoint(state, &input.command_id, execution)?;
            let response = paid_request(state, &call_id, &input.command_id, &bytes).await?;
            add_usage(&mut execution.usage, &response.usage);
            execution.exchanges.last_mut().ok_or(())?.response = Some(response.clone());
            execution.response = Some(response);
            execution.phase = "result".into();
            checkpoint(state, &input.command_id, execution)?;
            execution.project(evidence);
        }
        "result" => {}
        _ => return Err(()),
    }
    Ok(None)
}

fn prepare_continuation(
    execution: &mut ProviderLoop,
    evidence: &ProviderCallEvidence,
    outputs: Vec<(FunctionCall, String)>,
) -> Result<bool, ()> {
    let items = execution
        .request
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .ok_or(())?;
    if let Some(response) = &execution.response
        && !response.content.is_empty()
    {
        items.push(json!({"role":"assistant","content":response.content}));
    }
    for (call, output) in outputs {
        items.push(json!({"type":"function_call","call_id":call.call_id,"name":call.name,"arguments":call.arguments}));
        items.push(json!({"type":"function_call_output","call_id":call.call_id,"output":output}));
    }
    let remaining_input = evidence
        .reserved_input_tokens
        .saturating_sub(execution.usage.input_tokens.unwrap_or(u64::MAX));
    let remaining_output = evidence
        .reserved_output_tokens
        .saturating_sub(execution.usage.output_tokens.unwrap_or(u64::MAX));
    let remaining_total = evidence
        .reserved_total_tokens
        .saturating_sub(execution.usage.total_tokens.unwrap_or(u64::MAX));
    let model = execution
        .request
        .get("model")
        .and_then(Value::as_str)
        .ok_or(())?;
    let instructions = execution
        .request
        .get("instructions")
        .and_then(Value::as_str)
        .ok_or(())?;
    let items = execution
        .request
        .get("input")
        .and_then(Value::as_array)
        .ok_or(())?;
    let Ok(prepared) = prepare_items(
        model,
        instructions,
        items,
        remaining_input,
        remaining_output,
        remaining_total,
    ) else {
        return Ok(false);
    };
    execution.request = serde_json::from_slice(&prepared.bytes).map_err(|_| ())?;
    execution.round += 1;
    execution.phase = "ready".into();
    Ok(true)
}

fn invoke(execution: &mut ProviderLoop, call: &FunctionCall) -> Result<String, &'static str> {
    if call.arguments.len() > 4096
        || call.call_id.is_empty()
        || call.call_id.len() > 128
        || !call
            .call_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-'))
    {
        return Err("provider_tool_rejected");
    }
    let Some(source) = execution.sources.get(&call.name) else {
        return Err("provider_tool_rejected");
    };
    let input_digest = digest(call.arguments.as_bytes());
    if let Some(prior) = execution.tools.get(&call.call_id) {
        if prior.call != *call {
            return Err("provider_tool_rejected");
        }
        return if prior.state == "completed" {
            Ok(prior.output.clone())
        } else {
            Err("provider_tool_rejected")
        };
    }
    if execution.tools.len() >= 8 {
        return Err("provider_tool_limit");
    }
    let valid = serde_json::from_str::<Value>(&call.arguments)
        .ok()
        .is_some_and(|value| value.as_object().is_some_and(serde_json::Map::is_empty));
    let output = if valid {
        serde_json::to_string(source).map_err(|_| "provider_tool_rejected")?
    } else {
        String::new()
    };
    let reason = if !valid {
        Some("provider_tool_rejected")
    } else if output.len() > 32768 {
        Some("provider_tool_output_limit")
    } else {
        None
    };
    execution.tools.insert(
        call.call_id.clone(),
        RetainedTool {
            call: call.clone(),
            input_digest,
            output: if reason.is_none() {
                output.clone()
            } else {
                String::new()
            },
            state: if reason.is_none() {
                "completed"
            } else {
                "rejected"
            }
            .into(),
        },
    );
    reason.map_or(Ok(output), Err)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "fixture assertions fail immediately on unexpected state"
)]
mod tests {
    use super::*;
    fn execution() -> ProviderLoop {
        ProviderLoop {
            request: json!({}),
            phase: "ready".into(),
            round: 0,
            response: None,
            usage: TokenUsage::default(),
            sources: BTreeMap::from([(
                "read_workspace_instructions".into(),
                json!({"instructions":"frozen"}),
            )]),
            tools: BTreeMap::new(),
            exchanges: Vec::new(),
        }
    }
    fn call(id: &str, args: &str) -> FunctionCall {
        FunctionCall {
            call_id: id.into(),
            name: "read_workspace_instructions".into(),
            arguments: args.into(),
        }
    }
    #[test]
    fn scoped_invocation_replays_exact_output_and_rejects_changed_identity_or_scope() {
        let mut execution = execution();
        let request = call("same", "{}");
        let original = invoke(&mut execution, &request).unwrap();
        execution
            .sources
            .insert(request.name.clone(), json!({"instructions":"changed"}));
        assert_eq!(invoke(&mut execution, &request).unwrap(), original);
        assert_eq!(
            invoke(
                &mut execution,
                &call("same", "{\"conversation_id\":\"foreign\"}")
            ),
            Err("provider_tool_rejected")
        );
        assert_eq!(
            invoke(
                &mut execution,
                &call("foreign", "{\"project_id\":\"foreign\"}")
            ),
            Err("provider_tool_rejected")
        );
        assert_eq!(execution.tools["foreign"].state, "rejected");
    }
    #[test]
    fn invocation_count_argument_size_and_output_size_are_hard_bounds() {
        let mut execution = execution();
        for index in 0..8 {
            invoke(&mut execution, &call(&format!("call_{index}"), "{}")).unwrap();
        }
        assert_eq!(
            invoke(&mut execution, &call("ninth", "{}")),
            Err("provider_tool_limit")
        );
        let mut execution = super::tests::execution();
        assert_eq!(
            invoke(&mut execution, &call("large", &" ".repeat(4097))),
            Err("provider_tool_rejected")
        );
        execution.sources.insert(
            "read_workspace_instructions".into(),
            json!({"instructions":"x".repeat(32768)}),
        );
        assert_eq!(
            invoke(&mut execution, &call("output", "{}")),
            Err("provider_tool_output_limit")
        );
        assert!(execution.tools["output"].output.is_empty());
        let mut unknown = call("shell", "{}");
        unknown.name = "shell".into();
        assert_eq!(
            invoke(&mut execution, &unknown),
            Err("provider_tool_rejected")
        );
    }
    #[test]
    fn retained_quota_counts_utf8_bytes_for_loop_results_and_provider_evidence() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE provider_calls(request BLOB,evidence TEXT);CREATE TABLE results(result TEXT);CREATE TABLE provider_loops(command_id TEXT,state TEXT);").unwrap();
        let text = serde_json::to_string(&json!({"content":"é世界🦀".repeat(10)})).unwrap();
        connection
            .execute(
                "INSERT INTO provider_calls VALUES(?1,?2)",
                params![text.as_bytes(), text],
            )
            .unwrap();
        connection
            .execute("INSERT INTO results VALUES(?1)", [&text])
            .unwrap();
        connection
            .execute(
                "INSERT INTO provider_loops VALUES('retained',?1),('active',?1)",
                [&text],
            )
            .unwrap();
        assert_eq!(
            retained_bytes(&connection, "active").unwrap(),
            i64::try_from(text.len() * 4).unwrap()
        );
    }
}
