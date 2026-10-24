//! The model can request reads; only frozen session inputs grant their authority.
use super::*;
use ayni_orchestrator_events::v1::ToolInvocationEvidence;
use ayni_orchestrator_providers::responses::{FunctionCall, ResponseEvidence, TokenUsage};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize)]
struct RetainedTool {
    call: FunctionCall,
    input_digest: String,
    output: String,
    state: String,
    #[serde(default)]
    consent_id: Option<String>,
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
    #[serde(default)]
    mcp_tools: BTreeMap<String, super::mcp::Tool>,
}

impl ProviderLoop {
    pub(super) fn new(
        input: &PrepareInput,
        bytes: &[u8],
        mcp_tools: BTreeMap<String, super::mcp::Tool>,
    ) -> Result<Self, StatusCode> {
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
            sources.insert("read_project_status".into(),json!({"snapshot_id":snapshot.snapshot_id,"project_id":snapshot.project_id,"objective":snapshot.objective,"status":snapshot.status,"context_revision":snapshot.context_revision,"project_resource_version":snapshot.project_resource_version,"tasks":snapshot.tasks.iter().map(|fact|json!({"task_id":fact.task_id,"attempt_id":fact.attempt_id,"parent_attempt_id":fact.parent_attempt_id,"coding_state":fact.coding_state,"provider_state":fact.provider_state,"accepted_coding_state":fact.accepted_coding_state,"quality_state":fact.quality_state,"quality_artifact_digest":fact.quality_artifact_digest,"publication_state":fact.publication_state,"publication_id":fact.publication_id,"pull_request_url":fact.pull_request_url})).collect::<Vec<_>>(),"task_facts_available":snapshot.total_tasks.is_some(),"total_tasks":snapshot.total_tasks,"has_more_tasks":snapshot.has_more_tasks}));
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
            mcp_tools,
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
    if retained.saturating_add(i64::try_from(json.len() + 256 * 1024).map_err(|_| ())?)
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
    proposal: &mut Option<ayni_orchestrator_events::v1::PlanProposal>,
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
        if cancelled(state, &input.command_id) {
            evidence.state = "cancelled".into();
            return Ok((String::new(), "cancelled".into()));
        }
        if response
            .tool_calls
            .iter()
            .any(|call| call.name == "propose_plan")
        {
            return Ok(propose(&response, evidence, proposal));
        }
        if !input.allowed {
            return Ok(fail(evidence, "provider_interrupted"));
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
            let invoked = if execution.mcp_tools.contains_key(&call.name) {
                invoke_mcp(state, input, &mut execution, call).await
            } else {
                invoke(&mut execution, call)
            };
            match invoked {
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

fn propose(
    response: &ResponseEvidence,
    evidence: &mut ProviderCallEvidence,
    proposal: &mut Option<ayni_orchestrator_events::v1::PlanProposal>,
) -> (String, String) {
    let [call] = response.tool_calls.as_slice() else {
        return fail(evidence, "invalid_plan_proposal");
    };
    if call.call_id.is_empty()
        || call.call_id.len() > 128
        || !call
            .call_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || ayni_orchestrator_events::plan_proposals::validate_proposal(call.arguments.as_bytes())
            .is_err()
    {
        return fail(evidence, "invalid_plan_proposal");
    }
    *proposal = Some(ayni_orchestrator_events::v1::PlanProposal {
        invocation_id: call.call_id.clone(),
        proposal_json: call.arguments.as_bytes().to_vec(),
    });
    evidence.state = "completed".into();
    (
        if response.content.trim().is_empty() {
            "A plan proposal is ready for review.".into()
        } else {
            response.content.clone()
        },
        String::new(),
    )
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
    let Ok(prepared) = ayni_orchestrator_providers::responses::prepare_items_with_tools(
        model,
        instructions,
        items,
        remaining_input,
        remaining_output,
        remaining_total,
        execution.request.get("tools").ok_or(())?,
    ) else {
        return Ok(false);
    };
    execution.request = serde_json::from_slice(&prepared.bytes).map_err(|_| ())?;
    execution.round += 1;
    execution.phase = "ready".into();
    Ok(true)
}

fn consume_consent(
    input: &SendInput,
    execution: &ProviderLoop,
    tool: &super::mcp::Tool,
    arguments: &Value,
) -> Result<Option<String>, &'static str> {
    Ok(if tool.read_only {
        None
    } else {
        let consent = tool.consent.as_ref().ok_or("provider_tool_rejected")?;
        let argument_bytes = serde_json::to_vec(arguments).map_err(|_| "provider_tool_rejected")?;
        if consent.command_id != input.command_id
            || !consent.acknowledge_external_effect
            || consent.arguments_digest != digest(&argument_bytes)
        {
            return Err("provider_tool_rejected");
        }
        let id = digest(
            &serde_json::to_vec(&(
                input.command_id.as_str(),
                tool.alias.as_str(),
                &consent.arguments_digest,
            ))
            .map_err(|_| "provider_tool_rejected")?,
        );
        if execution
            .tools
            .values()
            .any(|prior| prior.consent_id.as_ref() == Some(&id))
        {
            return Err("provider_tool_rejected");
        }
        Some(id)
    })
}

async fn invoke_mcp(
    state: &WorkerState,
    input: &SendInput,
    execution: &mut ProviderLoop,
    call: &FunctionCall,
) -> Result<String, &'static str> {
    if call.arguments.len() > 4096
        || call.call_id.is_empty()
        || call.call_id.len() > 128
        || !call
            .call_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        return Err("provider_tool_rejected");
    }
    if let Some(prior) = execution.tools.get(&call.call_id) {
        if prior.call != *call {
            return Err("provider_tool_rejected");
        }
        return if prior.state == "completed" {
            Ok(prior.output.clone())
        } else if matches!(prior.state.as_str(), "sending" | "uncertain") {
            Err("provider_tool_uncertain")
        } else {
            Err("provider_tool_rejected")
        };
    }
    if execution.tools.len() >= 8 {
        return Err("provider_tool_limit");
    }
    let tool = execution
        .mcp_tools
        .get(&call.name)
        .ok_or("provider_tool_rejected")?
        .clone();
    let arguments: Value =
        serde_json::from_str(&call.arguments).map_err(|_| "provider_tool_rejected")?;
    if !super::mcp::accepts(&tool, &arguments) {
        return Err("provider_tool_rejected");
    }
    let consent_id = consume_consent(input, execution, &tool, &arguments)?;
    execution.tools.insert(
        call.call_id.clone(),
        RetainedTool {
            call: call.clone(),
            input_digest: digest(call.arguments.as_bytes()),
            output: String::new(),
            state: "sending".into(),
            consent_id,
        },
    );
    checkpoint(state, &input.command_id, execution).map_err(|()| "provider_tool_uncertain")?;
    let invocation = tokio::time::timeout(
        Duration::from_secs(tool.timeout_seconds),
        super::mcp::invoke(&tool, &call.call_id, arguments),
    );
    tokio::pin!(invocation);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let result = loop {
        tokio::select! {
            result = &mut invocation => break result.unwrap_or(Err("provider_tool_uncertain")),
            _ = interval.tick() => if cancelled(state, &input.command_id) { break Err("provider_tool_uncertain"); },
        }
    };
    // A mutation can commit before any error response. Every unretained result is uncertain.
    let result = if tool.read_only {
        result
    } else {
        result.map_err(|_| "provider_tool_uncertain")
    };
    let retained = execution
        .tools
        .get_mut(&call.call_id)
        .ok_or("provider_tool_rejected")?;
    match &result {
        Ok(output) => {
            retained.output.clone_from(output);
            retained.state = "completed".into();
        }
        Err("provider_tool_uncertain") => retained.state = "uncertain".into(),
        Err(_) => retained.state = "rejected".into(),
    }
    checkpoint(state, &input.command_id, execution).map_err(|()| "provider_tool_uncertain")?;
    result
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
            consent_id: None,
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
            mcp_tools: BTreeMap::new(),
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
    #[tokio::test]
    async fn ambiguous_mcp_invocation_replay_cannot_send_again() {
        let mut execution = execution();
        let call = FunctionCall {
            call_id: "mcp_same".into(),
            name: "mcp_lookup".into(),
            arguments: "{}".into(),
        };
        execution.tools.insert(
            call.call_id.clone(),
            RetainedTool {
                call: call.clone(),
                input_digest: digest(b"{}"),
                output: String::new(),
                state: "sending".into(),
                consent_id: None,
            },
        );
        let state = WorkerState {
            active: Mutex::new(std::collections::HashSet::new()),
            provider: Mutex::new(None),
            session_id: "session".into(),
            generation: 1,
            conversation_id: "conversation".into(),
            workspace_id: "workspace".into(),
            project_id: "project".into(),
            token: "token".into(),
            connection: Mutex::new(Connection::open_in_memory().unwrap()),
            last_request: Mutex::new(Instant::now()),
            idle_timeout: Mutex::new(Duration::from_secs(30)),
        };
        let input = SendInput {
            command_id: "command".into(),
            request_digest: "digest".into(),
            allowed: true,
            state: "sending".into(),
        };
        assert_eq!(
            invoke_mcp(&state, &input, &mut execution, &call).await,
            Err("provider_tool_uncertain")
        );
        let mut changed = call.clone();
        changed.arguments = "{\"changed\":true}".into();
        assert_eq!(
            invoke_mcp(&state, &input, &mut execution, &changed).await,
            Err("provider_tool_rejected")
        );
    }
    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "one retained-ledger external-effect scenario"
    )]
    #[ignore = "requires loopback networking; run scripts/ci/check-workflow-loopback.sh"]
    async fn exact_external_effect_consent_is_consumed_once_and_survives_ledger_reopen_requires_loopback()
     {
        use axum::{Json, Router, response::IntoResponse, routing::post};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let app = Router::new().route("/mcp", post(move |Json(body): Json<Value>| {
            let count = observed.clone();
            async move { match body["method"].as_str() {
                Some("initialize") => Json(json!({"jsonrpc":"2.0","id":"initialize","result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}}}})).into_response(),
                Some("notifications/initialized") => StatusCode::ACCEPTED.into_response(),
                Some("tools/list") => Json(json!({"jsonrpc":"2.0","id":"tools","result":{"tools":(["create","create500","large"].iter().map(|name|json!({"name":name,"inputSchema":{"type":"object","properties":{},"required":[],"additionalProperties":false}})).collect::<Vec<_>>())}})).into_response(),
                Some("tools/call") => { count.fetch_add(1, Ordering::SeqCst); if body["params"]["name"] == "create500" {return StatusCode::INTERNAL_SERVER_ERROR.into_response();} else if body["params"]["name"] == "large" {return Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":"x".repeat(5000)}})).into_response();} Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":[{"type":"text","text":"created"}]}})).into_response() },
                _ => StatusCode::BAD_REQUEST.into_response(),
            }}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let command_id = uuid::Uuid::now_v7().to_string();
        let state = WorkerState {
            active: Mutex::new(std::collections::HashSet::new()),
            provider: Mutex::new(None),
            session_id: "session".into(),
            generation: 1,
            conversation_id: "conversation".into(),
            workspace_id: "workspace".into(),
            project_id: "project".into(),
            token: "token".into(),
            connection: Mutex::new(Connection::open_in_memory().unwrap()),
            last_request: Mutex::new(Instant::now()),
            idle_timeout: Mutex::new(Duration::from_secs(30)),
        };
        state
            .connection
            .lock()
            .unwrap()
            .execute_batch(super::super::super::SCHEMA)
            .unwrap();
        state
            .connection
            .lock()
            .unwrap()
            .execute_batch(super::super::SCHEMA)
            .unwrap();
        state.connection.lock().unwrap().execute("INSERT INTO provider_calls(command_id,command_digest,request_digest,evidence,state) VALUES(?1,'digest','digest','{}','sending')",[&command_id]).unwrap();
        let mut execution = execution();
        execution.mcp_tools.insert("mcp_create".into(), super::super::mcp::Tool {
            alias:"mcp_create".into(),name:"create".into(),endpoint,description:"Create exact reviewed item".into(),input_schema:json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),read_only:false,max_output_bytes:4096,timeout_seconds:5,bearer_file:None,
            consent:Some(super::super::mcp::Consent {command_id:command_id.clone(),arguments_digest:digest(b"{}"),acknowledge_external_effect:true}),
        });
        state
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO provider_loops(command_id,state) VALUES(?1,?2)",
                params![command_id, serde_json::to_string(&execution).unwrap()],
            )
            .unwrap();
        let input = SendInput {
            command_id: command_id.clone(),
            request_digest: "digest".into(),
            allowed: true,
            state: "sending".into(),
        };
        let call = FunctionCall {
            call_id: "effect_1".into(),
            name: "mcp_create".into(),
            arguments: "{}".into(),
        };
        let result = invoke_mcp(&state, &input, &mut execution, &call)
            .await
            .unwrap();
        assert!(result.contains("created"));
        let json: String = state
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM provider_loops WHERE command_id=?1",
                [&command_id],
                |row| row.get(0),
            )
            .unwrap();
        let mut reopened: ProviderLoop = serde_json::from_str(&json).unwrap();
        assert_eq!(
            invoke_mcp(&state, &input, &mut reopened, &call)
                .await
                .unwrap(),
            result
        );
        let second = FunctionCall {
            call_id: "effect_2".into(),
            ..call.clone()
        };
        assert_eq!(
            invoke_mcp(&state, &input, &mut reopened, &second).await,
            Err("provider_tool_rejected")
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        for (index, name) in ["create500", "large"].iter().enumerate() {
            let mut tool = reopened.mcp_tools["mcp_create"].clone();
            tool.alias = format!("mcp_failure_{index}");
            tool.name = (*name).into();
            let failed = FunctionCall {
                call_id: format!("failed_{index}"),
                name: tool.alias.clone(),
                arguments: "{}".into(),
            };
            reopened.mcp_tools.insert(tool.alias.clone(), tool);
            assert_eq!(
                invoke_mcp(&state, &input, &mut reopened, &failed).await,
                Err("provider_tool_uncertain")
            );
            assert_eq!(reopened.tools[&failed.call_id].state, "uncertain");
            assert_eq!(
                invoke_mcp(&state, &input, &mut reopened, &failed).await,
                Err("provider_tool_uncertain")
            );
        }
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "post-commit errors cannot repeat effects"
        );
        server.abort();
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
