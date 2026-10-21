//! Bounded scoped reads and paid exchanges; a sending replay is never a new request.
use super::*;
use ayni_orchestrator_events::v1::{ConversationContextRecord, ProviderCallEvidence};
use ayni_orchestrator_providers::responses::{
    ContextMessage, ResponsesClient, prepare as prepare_request,
};

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS provider_calls(command_id TEXT PRIMARY KEY,command_digest TEXT NOT NULL,request BLOB,request_digest TEXT NOT NULL,evidence TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','sending','completed','failed','uncertain','cancelled')),cancelled INTEGER NOT NULL DEFAULT 0,settled INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS provider_loops(command_id TEXT PRIMARY KEY,state TEXT NOT NULL);";

#[derive(Deserialize, Serialize)]
pub struct PrepareInput {
    pub command_bytes: Vec<u8>,
    pub context: Vec<ConversationContextRecord>,
}

#[derive(Deserialize, Serialize)]
pub struct Preparation {
    pub call_id: String,
    pub request_digest: String,
    pub context_digest: String,
    pub reserved_input_tokens: u64,
    pub reserved_output_tokens: u64,
    pub reserved_total_tokens: u64,
    pub state: String,
    pub result: Option<ExecutionResult>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SendInput {
    pub command_id: String,
    pub request_digest: String,
    pub allowed: bool,
    pub state: String,
}

#[derive(Deserialize, Serialize)]
pub struct CancelInput {
    pub command_id: String,
    pub command_digest: String,
}

#[allow(
    clippy::suspicious_operation_groupings,
    reason = "executor and session generations must both match the retained session generation"
)]
fn scope_valid(
    state: &WorkerState,
    command: &ayni_orchestrator_events::v1::ConversationTurnRequested,
) -> bool {
    command.binding.as_ref().is_some_and(|binding| {
        binding.workspace_id == state.workspace_id
            && binding.project_id == state.project_id
            && binding.conversation_id == state.conversation_id
            && binding.executor_generation == state.generation
            && binding.execution_binding_revision == "openai-responses-v1"
    }) && command.session.as_ref().is_some_and(|scope| {
        scope.session_id == state.session_id && scope.generation == state.generation
    })
}

pub(super) async fn prepare(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<PrepareInput>,
) -> Result<axum::Json<Preparation>, StatusCode> {
    if !authorized(&state, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let PreparedCall {
        identity,
        command_digest,
        mut request_digest,
        context_digest,
        prepared,
        evidence,
    } = build_preparation(&state, &input)?;
    let mut connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let tx = connection
        .transaction()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT command_digest,request_digest FROM provider_calls WHERE command_id=?1",
            [&identity.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if let Some((prior_command, prior_request)) = existing {
        if prior_command != command_digest {
            return Err(StatusCode::CONFLICT);
        }
        request_digest = prior_request;
    } else {
        ensure_provider(&state, credential_client)?;
        tx.execute("DELETE FROM provider_calls WHERE state IN ('completed','failed','cancelled') AND settled=1 AND command_id IN (SELECT command_id FROM results WHERE acknowledged=1 AND acknowledged_at < unixepoch()-691200)", []).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        tx.execute("DELETE FROM provider_loops WHERE command_id NOT IN (SELECT command_id FROM provider_calls)", []).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        tx.execute("DELETE FROM results WHERE acknowledged=1 AND acknowledged_at < unixepoch()-691200 AND command_id NOT IN (SELECT command_id FROM provider_calls)", []).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let loop_json = serde_json::to_string(&ProviderLoop::new(&input, &prepared.bytes)?)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let json = serde_json::to_string(&evidence).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM provider_calls", [], |row| row.get(0))
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let retained: i64 = tx
            .query_row(
                "SELECT COALESCE(sum(COALESCE(length(request),0)+length(CAST(evidence AS BLOB))),0)+(SELECT COALESCE(sum(length(CAST(result AS BLOB))),0) FROM results)+(SELECT COALESCE(sum(length(CAST(state AS BLOB))),0) FROM provider_loops) FROM provider_calls",
                [],
                |row| row.get(0),
            )
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if count >= 50000
            || retained
                + i64::try_from(prepared.bytes.len() + loop_json.len() + json.len() + 64 * 1024)
                    .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?
                > MAX_RETAINED_BYTES
        {
            return Err(StatusCode::INSUFFICIENT_STORAGE);
        }
        tx.execute("INSERT INTO provider_calls(command_id,command_digest,request,request_digest,evidence,state) VALUES(?1,?2,?3,?4,?5,'prepared')",params![identity.id,command_digest,prepared.bytes,request_digest,json]).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        tx.execute(
            "INSERT INTO provider_loops(command_id,state) VALUES(?1,?2)",
            params![identity.id, loop_json],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    }
    let (retained_state, json): (String, String) = tx
        .query_row(
            "SELECT state,evidence FROM provider_calls WHERE command_id=?1",
            [&identity.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let result: Option<String> = tx
        .query_row(
            "SELECT result FROM results WHERE command_id=?1",
            [&identity.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let retained: ProviderCallEvidence =
        serde_json::from_str(&json).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tx.commit().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    drop(connection);
    Ok(axum::Json(Preparation {
        call_id: identity.id,
        request_digest,
        context_digest,
        reserved_input_tokens: retained.reserved_input_tokens,
        reserved_output_tokens: retained.reserved_output_tokens,
        reserved_total_tokens: retained.reserved_total_tokens,
        state: retained_state,
        result: result
            .map(|result| serde_json::from_str(&result))
            .transpose()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    }))
}

struct PreparedCall {
    identity: ayni_orchestrator_events::EventIdentity,
    command_digest: String,
    request_digest: String,
    context_digest: String,
    prepared: ayni_orchestrator_providers::responses::PreparedRequest,
    evidence: ProviderCallEvidence,
}

fn build_preparation(
    state: &WorkerState,
    input: &PrepareInput,
) -> Result<PreparedCall, StatusCode> {
    let (identity, _, command) =
        decode_command(&input.command_bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    if !scope_valid(state, &command) {
        return Err(StatusCode::FORBIDDEN);
    }
    let config = command
        .frozen_configuration
        .as_ref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    let manifest = command
        .context_manifest
        .as_ref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    if config.provider != "openai" || input.context.len() as u64 != manifest.record_count {
        return Err(StatusCode::BAD_REQUEST);
    }
    // The application supplies records only from the immutable eligibility manifest.
    // Verify their complete digest locally before deriving provider request bytes.
    let context_digest = context_digest(&input.context);
    if context_digest != manifest.digest {
        return Err(StatusCode::CONFLICT);
    }
    let command_digest = digest(&input.command_bytes);
    if let Some(retained) =
        retained_preparation(state, &identity, &command_digest, &context_digest)?
    {
        return Ok(retained);
    }
    let messages = input
        .context
        .iter()
        .map(|record| ContextMessage {
            role: record.role.clone(),
            content: record.content.clone(),
        })
        .collect::<Vec<_>>();
    let prepared = prepare_request(
        &config.model,
        &config.instructions,
        &messages,
        config.max_input_tokens,
        config.max_output_tokens,
        config.max_total_tokens,
    )
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let request_digest = digest(&prepared.bytes);
    let evidence = ProviderCallEvidence {
        call_id: identity.id.clone(),
        provider: "openai".into(),
        model: config.model.clone(),
        configuration_revision_id: config.revision_id.clone(),
        context_digest: context_digest.clone(),
        request_digest: request_digest.clone(),
        reserved_input_tokens: config.max_input_tokens,
        reserved_output_tokens: config.max_output_tokens,
        reserved_total_tokens: config.max_input_tokens + config.max_output_tokens,
        state: "prepared".into(),
        ..Default::default()
    };
    Ok(PreparedCall {
        identity,
        command_digest,
        request_digest,
        context_digest,
        prepared,
        evidence,
    })
}

fn retained_preparation(
    state: &WorkerState,
    identity: &ayni_orchestrator_events::EventIdentity,
    command_digest: &str,
    context_digest: &str,
) -> Result<Option<PreparedCall>, StatusCode> {
    let row: Option<(String, String, Option<Vec<u8>>, String)> = state.connection.lock()
        .map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.query_row(
            "SELECT command_digest,request_digest,request,evidence FROM provider_calls WHERE command_id=?1",
            [&identity.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
        .optional().map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    let Some((retained_command, request_digest, bytes, json)) = row else {
        return Ok(None);
    };
    if retained_command != command_digest {
        return Err(StatusCode::CONFLICT);
    }
    let evidence: ProviderCallEvidence =
        serde_json::from_str(&json).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Some(PreparedCall {
        identity: identity.clone(),
        command_digest: command_digest.into(),
        request_digest,
        context_digest: context_digest.into(),
        prepared: ayni_orchestrator_providers::responses::PreparedRequest {
            bytes: bytes.unwrap_or_default(),
            reserved_input_tokens: evidence.reserved_input_tokens,
            reserved_output_tokens: evidence.reserved_output_tokens,
        },
        evidence,
    }))
}

// Kept identical to the authoritative context-manifest digest contract.
fn context_digest(records: &[ConversationContextRecord]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ayni-conversation-context-v1");
    for record in records {
        for value in [
            record.message_id.as_bytes(),
            &record.sequence.to_be_bytes(),
            record.role.as_bytes(),
            record.content.as_bytes(),
        ] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value);
        }
    }
    format!("sha256:{:x}", hash.finalize())
}

pub(super) async fn send(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<SendInput>,
) -> Result<axum::Json<ExecutionResult>, StatusCode> {
    if !authorized(&state, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _active = ActiveCall::enter(state.clone(), &input.command_id)?;
    let SendState {
        request,
        evidence,
        command_digest,
        send_allowed,
        allow_new_requests,
        replay,
    } = begin_send(&state, &input)?;
    if let Some(result) = replay {
        return Ok(axum::Json(result));
    }
    let (evidence, response_content, failure_reason) = if send_allowed {
        let mut authorized_input = input.clone();
        authorized_input.allowed = allow_new_requests;
        perform_request(&state, &authorized_input, &request, evidence).await
    } else {
        (evidence, String::new(), String::new())
    };
    retain_result(
        &state,
        &input,
        request.len(),
        &evidence,
        command_digest,
        response_content,
        failure_reason,
    )
}

struct SendState {
    request: Vec<u8>,
    evidence: ProviderCallEvidence,
    command_digest: String,
    send_allowed: bool,
    allow_new_requests: bool,
    replay: Option<ExecutionResult>,
}

fn begin_send(state: &WorkerState, input: &SendInput) -> Result<SendState, StatusCode> {
    let mut connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let tx = connection
        .transaction()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let (retained_digest,request,json,prior_state,command_digest,cancelled):(String,Option<Vec<u8>>,String,String,String,bool)=tx.query_row("SELECT request_digest,request,evidence,state,command_digest,cancelled FROM provider_calls WHERE command_id=?1",[&input.command_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional().map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.ok_or(StatusCode::NOT_FOUND)?;
    if retained_digest != input.request_digest {
        return Err(StatusCode::CONFLICT);
    }
    if let Some(json) = tx
        .query_row(
            "SELECT result FROM results WHERE command_id=?1",
            [&input.command_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    {
        let result: ExecutionResult =
            serde_json::from_str(&json).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        return Ok(SendState {
            request: Vec::new(),
            evidence: ProviderCallEvidence::default(),
            command_digest,
            send_allowed: false,
            allow_new_requests: false,
            replay: Some(result),
        });
    }
    let mut evidence: ProviderCallEvidence =
        serde_json::from_str(&json).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Completed exchange checkpoints can reconstruct a terminal outcome without
    // sending again. No replay grants authority for another paid exchange.
    let has_checkpoint: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_loops WHERE command_id=?1)",
            [&input.command_id],
            |row| row.get(0),
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let send_allowed = (prior_state == "prepared" && input.allowed && !cancelled)
        || (prior_state == "sending" && has_checkpoint && !cancelled);
    let allow_new_requests = prior_state == "prepared" && input.allowed && !cancelled;
    evidence.state = if send_allowed {
        "sending"
    } else if prior_state == "prepared" && (cancelled || input.state == "cancelled") {
        "cancelled"
    } else {
        "uncertain"
    }
    .into();
    tx.execute(
        "UPDATE provider_calls SET state=?2,evidence=?3 WHERE command_id=?1",
        params![
            input.command_id,
            evidence.state,
            serde_json::to_string(&evidence).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        ],
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tx.commit().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    drop(connection);
    Ok(SendState {
        request: request.unwrap_or_default(),
        evidence,
        command_digest,
        send_allowed,
        allow_new_requests,
        replay: None,
    })
}

async fn perform_request(
    state: &WorkerState,
    input: &SendInput,
    request: &[u8],
    mut evidence: ProviderCallEvidence,
) -> (ProviderCallEvidence, String, String) {
    if let Ok((content, reason)) = tools::execute(state, input, request, &mut evidence).await {
        (evidence, content, reason)
    } else {
        evidence.state = "uncertain".into();
        // Earlier completed exchanges do not establish usage for a later
        // exchange whose external outcome is unconfirmed.
        evidence.input_tokens = None;
        evidence.output_tokens = None;
        evidence.total_tokens = None;
        (evidence, String::new(), String::new())
    }
}

mod tools;
use tools::ProviderLoop;

fn retain_result(
    state: &WorkerState,
    input: &SendInput,
    request_len: usize,
    evidence: &ProviderCallEvidence,
    command_digest: String,
    response_content: String,
    failure_reason: String,
) -> Result<axum::Json<ExecutionResult>, StatusCode> {
    let result = ExecutionResult {
        command_id: input.command_id.clone(),
        command_digest,
        response_content,
        failure_reason: if evidence.state == "failed" {
            failure_reason
        } else {
            match evidence.state.as_str() {
                "completed" => "",
                "cancelled" => "cancelled",
                _ => "provider_uncertain",
            }
            .into()
        },
        provider_call: Some(evidence.clone()),
    };
    let json = serde_json::to_string(&result).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let tx = connection
        .transaction()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tx.execute(
        "UPDATE provider_calls SET state=?2,evidence=?3,settled=?4 WHERE command_id=?1",
        params![
            input.command_id,
            evidence.state,
            serde_json::to_string(&evidence).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
            evidence.total_tokens.is_some()
                || (evidence.state == "cancelled" && evidence.response_id.is_empty())
        ],
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tx.execute("INSERT OR IGNORE INTO results(command_id,digest,result,retained_bytes) VALUES(?1,?2,?3,?4)",params![input.command_id,result.command_digest,json,i64::try_from(request_len+json.len()).map_err(|_|StatusCode::PAYLOAD_TOO_LARGE)?]).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    tx.commit().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    drop(connection);
    *state
        .last_request
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)? = Instant::now();
    Ok(axum::Json(result))
}

fn ensure_provider(
    state: &WorkerState,
    load: impl FnOnce() -> Result<ResponsesClient, ()>,
) -> Result<(), StatusCode> {
    let mut provider = state
        .provider
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if provider.is_none() {
        *provider = Some(load().map_err(|()| StatusCode::FAILED_DEPENDENCY)?);
    }
    drop(provider);
    Ok(())
}

pub(super) fn credential_client() -> Result<ResponsesClient, ()> {
    let path = env::var("ORCHESTRATOR_OPENAI_API_KEY_PATH").map_err(|_| ())?;
    if !std::path::Path::new(&path).is_absolute() {
        return Err(());
    }
    let key = fs::read_to_string(path).map_err(|_| ())?;
    let endpoint =
        env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
    ResponsesClient::new(key.trim_end_matches(['\n', '\r']), &endpoint).map_err(|_| ())
}

pub(super) async fn cancel(
    State(state): State<Arc<WorkerState>>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<CancelInput>,
) -> Result<StatusCode, StatusCode> {
    if !authorized(&state, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let connection = state
        .connection
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let changed = connection
        .execute(
            "UPDATE provider_calls SET cancelled=1 WHERE command_id=?1 AND command_digest=?2",
            params![input.command_id, input.command_digest],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    drop(connection);
    if changed == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

struct ActiveCall {
    state: Arc<WorkerState>,
    command_id: String,
}
impl ActiveCall {
    fn enter(state: Arc<WorkerState>, command_id: &str) -> Result<Self, StatusCode> {
        if !state
            .active
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .insert(command_id.to_owned())
        {
            return Err(StatusCode::CONFLICT);
        }
        Ok(Self {
            state,
            command_id: command_id.to_owned(),
        })
    }
}
impl Drop for ActiveCall {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.active.lock() {
            active.remove(&self.command_id);
        }
    }
}

#[cfg(test)]
#[path = "provider_session_tests.rs"]
mod tests;
