//! Foreground Responses requests with explicit bounds and no automatic retry.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

/// Frozen provider input. Roles are supplied by accepted conversation history.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextMessage {
    pub role: String,
    pub content: String,
}

/// Exact request bytes and the conservative token reservation owned by one call.
#[derive(Clone, Debug)]
pub struct PreparedRequest {
    pub bytes: Vec<u8>,
    pub reserved_input_tokens: u64,
    pub reserved_output_tokens: u64,
}

/// A nullable usage value always means unknown, never zero.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

/// Bounded evidence retained before any application report is published.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResponseEvidence {
    pub response_id: String,
    pub model: String,
    pub content: String,
    pub usage: TokenUsage,
    pub failure_reason: String,
    /// Validated function-call envelopes only; provider reasoning is discarded.
    #[serde(default)]
    pub tool_calls: Vec<FunctionCall>,
}

/// A model request has no authority until the session validates it against its frozen scope.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// Scoped reads and terminal draft proposals. Parameters cannot select another scope.
#[must_use]
pub fn scoped_tools() -> Value {
    let mut tools = [
        ("read_workspace_instructions", "Read this turn's frozen workspace instructions."),
        ("read_project_status", "Read the originating project's retained factual status snapshot."),
        ("read_conversation_history", "Read accepted originating-conversation history through the frozen eligibility boundary."),
    ].into_iter().map(|(name, description)| json!({
        "type":"function","name":name,"description":description,"strict":true,
        "parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false}
    })).collect::<Vec<_>>();
    tools.push(crate::plan_proposals::tool());
    Value::Array(tools)
}

/// Safe classifications deliberately omit raw provider diagnostics and request data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseError {
    Configuration,
    InputLimit,
    Uncertain,
}

/// Construct one complete client-managed request without provider conversation state.
///
/// # Errors
/// Rejects unknown roles, implicit models and inputs outside the admitted allowance.
pub fn prepare(
    model: &str,
    instructions: &str,
    messages: &[ContextMessage],
    max_input: u64,
    max_output: u64,
    max_total: u64,
) -> Result<PreparedRequest, ResponseError> {
    if model.is_empty() || model.len() > 128 || max_output < 16 || messages.is_empty() {
        return Err(ResponseError::Configuration);
    }
    if messages
        .iter()
        .any(|message| !matches!(message.role.as_str(), "user" | "assistant"))
    {
        return Err(ResponseError::Configuration);
    }
    let input = messages
        .iter()
        .map(|message| json!({"role":message.role,"content":message.content}))
        .collect::<Vec<_>>();
    prepare_items(
        model,
        instructions,
        &input,
        max_input,
        max_output,
        max_total,
    )
}

/// Prepare a stateless continuation using only previously validated input items.
///
/// # Errors
/// Rejects a request that exceeds the remaining cumulative allowance.
pub fn prepare_items(
    model: &str,
    instructions: &str,
    input: &[Value],
    max_input: u64,
    max_output: u64,
    max_total: u64,
) -> Result<PreparedRequest, ResponseError> {
    prepare_items_with_tools(
        model,
        instructions,
        input,
        max_input,
        max_output,
        max_total,
        &scoped_tools(),
    )
}

/// Prepare a request with the exact retained tool allowlist and cumulative budgets.
/// # Errors
/// Rejects non-array tool definitions and requests outside the frozen allowance.
#[allow(
    clippy::too_many_arguments,
    reason = "explicit frozen request limits and tools"
)]
pub fn prepare_items_with_tools(
    model: &str,
    instructions: &str,
    input: &[Value],
    max_input: u64,
    max_output: u64,
    max_total: u64,
    tools: &Value,
) -> Result<PreparedRequest, ResponseError> {
    if !tools.is_array() {
        return Err(ResponseError::Configuration);
    }
    // Count all serialized tool definitions, call envelopes and outputs, including
    // escaped UTF-8, with conservative framing. Never truncate an authorized read.
    let input_bound = u64::try_from(
        serde_json::to_vec(&json!({
            "instructions":instructions,"input":input,"tools":tools
        }))
        .map_err(|_| ResponseError::Configuration)?
        .len(),
    )
    .map_err(|_| ResponseError::InputLimit)?
    .saturating_add(1024);
    let remaining = max_total
        .checked_sub(input_bound)
        .ok_or(ResponseError::InputLimit)?;
    let output = max_output.min(remaining);
    if input_bound > max_input || output < 16 {
        return Err(ResponseError::InputLimit);
    }
    let bytes = serde_json::to_vec(&json!({
        "model":model,"instructions":instructions,"input":input,
        "tools":tools,
        "max_output_tokens":output,"store":false,"background":false,
        "stream":false,"truncation":"disabled"
    }))
    .map_err(|_| ResponseError::Configuration)?;
    Ok(PreparedRequest {
        bytes,
        reserved_input_tokens: input_bound,
        reserved_output_tokens: output,
    })
}

/// Provider connection confined to one isolated session. Credentials never enter evidence.
#[derive(Clone)]
pub struct ResponsesClient {
    http: reqwest::Client,
    endpoint: url::Url,
    api_key: String,
    provider: String,
}

impl ResponsesClient {
    /// Construct a bounded client with redirects and retries disabled.
    ///
    /// # Errors
    /// Returns a safe configuration error, never a credential-bearing URL or HTTP error.
    pub fn new(api_key: &str, base_url: &str) -> Result<Self, ResponseError> {
        Self::for_provider("openai", api_key, base_url)
    }

    /// Construct a selected provider with the same no-retry transport boundary.
    /// # Errors
    /// Rejects unsupported providers, malformed endpoints and invalid credentials.
    pub fn for_provider(
        provider: &str,
        api_key: &str,
        base_url: &str,
    ) -> Result<Self, ResponseError> {
        if !matches!(provider, "openai" | "anthropic") {
            return Err(ResponseError::Configuration);
        }
        let endpoint = url::Url::parse(base_url).map_err(|_| ResponseError::Configuration)?;
        if api_key.trim().is_empty()
            || api_key.len() > 4096
            || api_key.chars().any(char::is_control)
            || !matches!(endpoint.scheme(), "https" | "http")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || base_url.split_once("://").is_none_or(|(_, rest)| {
                rest.split('/')
                    .next()
                    .is_none_or(|authority| authority.contains('@'))
            })
        {
            return Err(ResponseError::Configuration);
        }
        let resource = if provider == "anthropic" {
            "messages"
        } else {
            "responses"
        };
        let endpoint = url::Url::parse(&format!("{}/{resource}", base_url.trim_end_matches('/')))
            .map_err(|_| ResponseError::Configuration)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ResponseError::Configuration)?;
        Ok(Self {
            http,
            endpoint,
            api_key: api_key.to_owned(),
            provider: provider.to_owned(),
        })
    }

    /// Send exactly once. Every failure after sending intent is conservatively uncertain.
    ///
    /// # Errors
    /// All HTTP, transport, malformed-response and size failures retain uncertainty.
    pub async fn send(
        &self,
        call_id: &str,
        bytes: &[u8],
    ) -> Result<ResponseEvidence, ResponseError> {
        let wire = if self.provider == "anthropic" {
            crate::anthropic::request(bytes)?
        } else {
            bytes.to_owned()
        };
        let mut request = self
            .http
            .post(self.endpoint.clone())
            .header("content-type", "application/json")
            .header("X-Client-Request-Id", call_id);
        if self.provider == "anthropic" {
            request = request
                .header("anthropic-version", "2023-06-01")
                .header("x-api-key", &self.api_key);
        } else {
            request = request.bearer_auth(&self.api_key);
        }
        let mut response = request
            .body(wire)
            .send()
            .await
            .map_err(|_| ResponseError::Uncertain)?;
        if !response.status().is_success() {
            return Err(ResponseError::Uncertain);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ResponseError::Uncertain)?
        {
            if body.len() + chunk.len() > 256 * 1024 {
                return Err(ResponseError::Uncertain);
            }
            body.extend_from_slice(&chunk);
        }
        if self.provider == "anthropic" {
            crate::anthropic::response(&body)
        } else {
            parse_response(&body)
        }
    }
}

fn parse_response(bytes: &[u8]) -> Result<ResponseEvidence, ResponseError> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| ResponseError::Uncertain)?;
    let text = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
            })
            .map(str::to_owned)
            .ok_or(ResponseError::Uncertain)
    };
    let response_id = text("id")?;
    let model = text("model")?;
    let mut tool_calls = Vec::new();
    let (content, failure_reason) = match body.get("status").and_then(Value::as_str) {
        Some("completed") => match response_content(&body, &mut tool_calls) {
            Ok(content) => (content, String::new()),
            Err(reason) => (String::new(), reason.into()),
        },
        Some("incomplete") => (
            String::new(),
            if body
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
                == Some("max_output_tokens")
            {
                "provider_output_limit"
            } else {
                "provider_rejected"
            }
            .into(),
        ),
        Some("failed") => (String::new(), "provider_rejected".into()),
        _ => return Err(ResponseError::Uncertain),
    };
    let usage = body.get("usage");
    let count = |name: &str| {
        usage
            .and_then(|usage| usage.get(name))
            .and_then(Value::as_u64)
    };
    Ok(ResponseEvidence {
        response_id,
        model,
        content,
        failure_reason,
        tool_calls,
        usage: TokenUsage {
            input_tokens: count("input_tokens"),
            output_tokens: count("output_tokens"),
            total_tokens: count("total_tokens"),
        },
    })
}

fn response_content(
    body: &Value,
    tool_calls: &mut Vec<FunctionCall>,
) -> Result<String, &'static str> {
    let outputs = body
        .get("output")
        .and_then(Value::as_array)
        .ok_or("provider_rejected")?;
    let mut content = String::new();
    let mut reasoning = false;
    for output in outputs {
        match output.get("type").and_then(Value::as_str) {
            Some("message") => {
                if output.get("role").and_then(Value::as_str) != Some("assistant") {
                    return Err("provider_rejected");
                }
                for item in output
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or("provider_rejected")?
                {
                    if item.get("type").and_then(Value::as_str) != Some("output_text") {
                        return Err("provider_rejected");
                    }
                    content.push_str(
                        item.get("text")
                            .and_then(Value::as_str)
                            .ok_or("provider_rejected")?,
                    );
                    if content.len() > 16384 {
                        return Err("provider_output_limit");
                    }
                }
            }
            Some("reasoning") => reasoning = true, // Never retain or project reasoning.
            Some("function_call") => {
                let field = |name: &str, max: usize| {
                    output
                        .get(name)
                        .and_then(Value::as_str)
                        .filter(|value| {
                            !value.is_empty()
                                && value.len() <= max
                                && !value.chars().any(char::is_control)
                        })
                        .map(str::to_owned)
                        .ok_or("provider_tool_rejected")
                };
                let call = FunctionCall {
                    call_id: field("call_id", 128)?,
                    name: field("name", 128)?,
                    arguments: output
                        .get("arguments")
                        .and_then(Value::as_str)
                        .filter(|value| {
                            value.len()
                                <= if output.get("name").and_then(Value::as_str)
                                    == Some("propose_plan")
                                {
                                    40960
                                } else {
                                    4096
                                }
                        })
                        .ok_or("provider_tool_rejected")?
                        .to_owned(),
                };
                if tool_calls.len() >= 8 {
                    return Err("provider_tool_limit");
                }
                tool_calls.push(call);
            }
            _ => return Err("provider_rejected"),
        }
    }
    // Reasoning models require these items on continuation. This execution
    // profile deliberately does not retain them, so fail before running tools.
    if reasoning && !tool_calls.is_empty() {
        return Err("provider_tool_rejected");
    }
    if content.is_empty() && tool_calls.is_empty() {
        return Err("provider_rejected");
    }
    if content
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err("provider_rejected");
    }
    Ok(content)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "fixture construction and assertions fail immediately on unexpected state"
)]
mod tests {
    use super::*;
    #[test]
    fn request_is_client_managed_and_exposes_only_scoped_reads_and_draft_proposals() {
        let request = prepare(
            "explicit-model",
            "frozen instructions",
            &[ContextMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            8192,
            512,
            8704,
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&request.bytes).unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["background"], false);
        assert_eq!(body["truncation"], "disabled");
        assert!(body.get("previous_response_id").is_none());
        assert!(body.get("conversation").is_none());
        assert_eq!(body["tools"], scoped_tools());
        assert!(
            prepare(
                "explicit-model",
                "frozen instructions",
                &[ContextMessage {
                    role: "user".into(),
                    content: "hello".into()
                }],
                100,
                512,
                2560
            )
            .is_err()
        );
    }
    #[test]
    fn missing_usage_stays_unknown_and_reasoning_is_not_output() {
        let response = parse_response(br#"{"id":"resp_test","model":"model","status":"completed","output":[{"type":"reasoning","summary":[{"text":"private"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}]}"#).unwrap();
        assert_eq!(response.content, "answer");
        assert_eq!(response.usage, TokenUsage::default());
        assert_eq!(parse_response(br#"{"id":"resp_test","model":"model","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[],"usage":{"input_tokens":4,"output_tokens":16,"total_tokens":20}}"#).unwrap().failure_reason,"provider_output_limit");
    }
}
