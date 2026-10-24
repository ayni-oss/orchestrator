//! Scoped deployment grants, frozen per request; external effects require exact one-use consent.
use super::*;
use ayni_orchestrator_providers::responses::{PreparedRequest, prepare_items_with_tools};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tool {
    pub alias: String,
    pub name: String,
    pub endpoint: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
    pub max_output_bytes: usize,
    pub timeout_seconds: u64,
    pub bearer_file: Option<String>,
    #[serde(default)]
    pub consent: Option<Consent>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Consent {
    pub command_id: String,
    pub arguments_digest: String,
    pub acknowledge_external_effect: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    workspace_id: String,
    project_id: String,
    configuration_revision_id: String,
    tools: Vec<Tool>,
}

pub(super) fn prepare(
    state: &WorkerState,
    config: &ayni_orchestrator_events::v1::AgentExecutionConfiguration,
    command_id: &str,
    request: PreparedRequest,
) -> Result<(PreparedRequest, BTreeMap<String, Tool>), StatusCode> {
    let Ok(path) = env::var("ORCHESTRATOR_MCP_CONFIG_PATH") else {
        return Ok((request, BTreeMap::new()));
    };
    // Deployment supplies this immutable, workspace-scoped volume; model text cannot select a path.
    if path != "/var/run/mcp-config/grants.json" {
        return Err(StatusCode::FORBIDDEN);
    }
    let bytes = fs::read(path).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if bytes.len() > 65_536 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let grants: Vec<Grant> = serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut selected = BTreeMap::new();
    for grant in grants {
        let revision_matches = grant.configuration_revision_id == config.revision_id;
        if grant.workspace_id != state.workspace_id
            || grant.project_id != state.project_id
            || !revision_matches
        {
            continue;
        }
        for tool in grant.tools {
            if !tool.read_only
                && tool
                    .consent
                    .as_ref()
                    .is_none_or(|consent| consent.command_id != command_id)
            {
                continue;
            }
            if !valid_tool(&tool) || selected.insert(tool.alias.clone(), tool).is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }
    if selected.len() > 5 {
        return Err(StatusCode::BAD_REQUEST);
    }
    if selected.is_empty() {
        return Ok((request, selected));
    }
    let body: Value =
        serde_json::from_slice(&request.bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut tools = body["tools"]
        .as_array()
        .ok_or(StatusCode::BAD_REQUEST)?
        .clone();
    tools.extend(selected.values().map(|tool| json!({"type":"function","name":tool.alias,"description":tool.description,"parameters":tool.input_schema,"strict":true})));
    let prepared = prepare_items_with_tools(
        &config.model,
        &config.instructions,
        body["input"].as_array().ok_or(StatusCode::BAD_REQUEST)?,
        config.max_input_tokens,
        config.max_output_tokens,
        config.max_total_tokens,
        &Value::Array(tools),
    )
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok((prepared, selected))
}

fn valid_tool(tool: &Tool) -> bool {
    let url = reqwest::Url::parse(&tool.endpoint).ok();
    let endpoint = url.as_ref().is_some_and(|url| {
        (url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });
    tool.alias.starts_with("mcp_")
        && tool.alias.len() <= 64
        && tool
            .alias
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && !tool.name.is_empty()
        && tool.name.len() <= 128
        && tool
            .name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
        && (tool.read_only
            || tool.consent.as_ref().is_some_and(|consent| {
                uuid::Uuid::parse_str(&consent.command_id).is_ok()
                    && consent.acknowledge_external_effect
                    && consent.arguments_digest.len() == 71
                    && consent.arguments_digest.starts_with("sha256:")
                    && consent.arguments_digest[7..]
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit())
            }))
        && endpoint
        && tool.description.len() <= 1024
        && (1..=32768).contains(&tool.max_output_bytes)
        && (1..=30).contains(&tool.timeout_seconds)
        && tool.bearer_file.as_ref().is_none_or(|path| {
            path.starts_with("/var/run/mcp/")
                && path.len() > 13
                && path[13..]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        })
        && tool.input_schema["type"] == "object"
        && schema_valid(&tool.input_schema, 0)
}

// A deliberately bounded JSON-schema subset prevents silently ignoring constraints.
fn schema_valid(schema: &Value, depth: u8) -> bool {
    if depth > 5 || schema.to_string().len() > 8192 {
        return false;
    }
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object.keys().any(|key| {
        ![
            "type",
            "properties",
            "required",
            "additionalProperties",
            "items",
            "maxItems",
            "enum",
            "description",
        ]
        .contains(&key.as_str())
    }) {
        return false;
    }
    if let Some(values) = schema.get("enum") {
        let Some(values) = values.as_array() else {
            return false;
        };
        if values.is_empty()
            || values.len() > 32
            || values.iter().any(|value| !arguments_valid(schema, value))
        {
            return false;
        }
    }
    if schema
        .get("description")
        .is_some_and(|description| description.as_str().is_none_or(|text| text.len() > 1024))
    {
        return false;
    }
    let allowed = match schema["type"].as_str() {
        Some("object") => &[
            "type",
            "properties",
            "required",
            "additionalProperties",
            "enum",
            "description",
        ][..],
        Some("array") => &["type", "items", "maxItems", "enum", "description"][..],
        _ => &["type", "enum", "description"][..],
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return false;
    }
    match schema["type"].as_str() {
        Some("object") => {
            schema["additionalProperties"] == false
                && schema["properties"].as_object().is_some_and(|props| {
                    props.len() <= 16 && props.values().all(|value| schema_valid(value, depth + 1))
                })
                && schema["required"].as_array().is_some_and(|keys| {
                    let mut unique = std::collections::HashSet::new();
                    keys.len() <= 16
                        && keys.iter().all(|key| {
                            key.as_str().is_some_and(|key| {
                                unique.insert(key) && schema["properties"].get(key).is_some()
                            })
                        })
                })
        }
        Some("array") => {
            schema["maxItems"].as_u64().is_some_and(|n| n <= 32)
                && schema_valid(&schema["items"], depth + 1)
        }
        Some("string" | "integer" | "number" | "boolean" | "null") => true,
        _ => false,
    }
}
fn arguments_valid(schema: &Value, value: &Value) -> bool {
    if let Some(allowed) = schema["enum"].as_array()
        && !allowed.contains(value)
    {
        return false;
    }
    match schema["type"].as_str() {
        Some("object") => value.as_object().is_some_and(|object| {
            object.iter().all(|(key, value)| {
                schema["properties"]
                    .get(key)
                    .is_some_and(|property| arguments_valid(property, value))
            }) && schema["required"].as_array().is_some_and(|required| {
                required
                    .iter()
                    .all(|key| key.as_str().is_some_and(|key| object.contains_key(key)))
            })
        }),
        Some("array") => value.as_array().is_some_and(|items| {
            u64::try_from(items.len()).unwrap_or(u64::MAX)
                <= schema["maxItems"].as_u64().unwrap_or(0)
                && items
                    .iter()
                    .all(|item| arguments_valid(&schema["items"], item))
        }),
        Some("string") => value.is_string(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("number") => value.is_number(),
        Some("boolean") => value.is_boolean(),
        Some("null") => value.is_null(),
        _ => false,
    }
}

pub(super) fn accepts(tool: &Tool, arguments: &Value) -> bool {
    arguments_valid(&tool.input_schema, arguments)
}

/// JSON-response Streamable HTTP profile; redirects, SSE and server requests fail closed.
/// Each invocation owns its own initialized session and never automatically retries a call.
pub(super) async fn invoke(
    tool: &Tool,
    id: &str,
    arguments: Value,
) -> Result<String, &'static str> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(tool.timeout_seconds))
        .build()
        .map_err(|_| "provider_tool_rejected")?;
    let bearer = tool
        .bearer_file
        .as_ref()
        .map(fs::read_to_string)
        .transpose()
        .map_err(|_| "provider_tool_rejected")?;
    let request = |body: &Value, session: Option<&str>| {
        let mut request = client
            .post(&tool.endpoint)
            .header("Accept", "application/json, text/event-stream")
            .json(body);
        if let Some(bearer) = &bearer {
            request = request.bearer_auth(bearer.trim());
        }
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
    };
    let initialized = request(&json!({"jsonrpc":"2.0","id":"initialize","method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"ayni-orchestrator","version":"0.1.0"}}}),None)
        .send().await.map_err(|_| "provider_tool_uncertain")?;
    let session = initialized
        .headers()
        .get("Mcp-Session-Id")
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| "provider_tool_rejected")?;
    if session.as_ref().is_some_and(|session| {
        session.len() > 256 || session.bytes().any(|c| !(0x21..=0x7e).contains(&c))
    }) {
        return Err("provider_tool_rejected");
    }
    let result = bounded_json(initialized, 32768).await?;
    if result["jsonrpc"] != "2.0"
        || result["id"] != "initialize"
        || result["result"]["protocolVersion"] != "2025-03-26"
        || !result["result"]["capabilities"]["tools"].is_object()
    {
        return Err("provider_tool_rejected");
    }
    let notification = request(
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        session.as_deref(),
    )
    .send()
    .await
    .map_err(|_| "provider_tool_uncertain")?;
    if notification.status() != reqwest::StatusCode::ACCEPTED {
        return Err("provider_tool_rejected");
    }
    let discovery = request(
        &json!({"jsonrpc":"2.0","id":"tools","method":"tools/list","params":{}}),
        session.as_deref(),
    )
    .send()
    .await
    .map_err(|_| "provider_tool_uncertain")?;
    let discovery = bounded_json(discovery, 32768).await?;
    if discovery["jsonrpc"] != "2.0"
        || discovery["id"] != "tools"
        || !discovery["result"]["tools"]
            .as_array()
            .is_some_and(|tools| {
                tools.len() <= 64
                    && tools.iter().any(|remote| {
                        remote["name"] == tool.name && remote["inputSchema"] == tool.input_schema
                    })
            })
    {
        return Err("provider_tool_rejected");
    }
    let response = request(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":tool.name,"arguments":arguments}}),session.as_deref()).send().await.map_err(|_| "provider_tool_uncertain")?;
    let result = bounded_json(response, tool.max_output_bytes).await?;
    if result["jsonrpc"] != "2.0"
        || result["id"] != id
        || result.get("error").is_some()
        || result["result"].is_null()
    {
        return Err("provider_tool_rejected");
    }
    serde_json::to_string(&result["result"]).map_err(|_| "provider_tool_rejected")
}
async fn bounded_json(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Value, &'static str> {
    if !response.status().is_success()
        || !response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"))
    {
        return Err("provider_tool_rejected");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "provider_tool_uncertain")?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("provider_tool_output_limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "provider_tool_rejected")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool() -> Tool {
        Tool {
            alias: "mcp_project_lookup".into(),
            name: "project_lookup".into(),
            endpoint: "https://example.invalid/mcp".into(),
            description: "Read the configured project.".into(),
            input_schema: json!({"type":"object","properties":{"project":{"type":"string","enum":["frozen-project"]}},"required":["project"],"additionalProperties":false}),
            read_only: true,
            max_output_bytes: 4096,
            timeout_seconds: 5,
            bearer_file: None,
            consent: None,
        }
    }
    #[test]
    fn scoped_schema_and_transport_reject_expansion_and_mutations() {
        let mut tool = tool();
        assert!(valid_tool(&tool));
        assert!(accepts(&tool, &json!({"project":"frozen-project"})));
        assert!(!accepts(&tool, &json!({"project":"foreign"})));
        assert!(!accepts(
            &tool,
            &json!({"project":"frozen-project","shell":"anything"})
        ));
        tool.read_only = false;
        assert!(!valid_tool(&tool));
        tool.read_only = true;
        tool.endpoint = "https://user:password@example.invalid/mcp".into();
        assert!(!valid_tool(&tool));
        tool.endpoint = "http://example.invalid/mcp".into();
        assert!(!valid_tool(&tool));
        tool.endpoint = "https://example.invalid/mcp".into();
        tool.bearer_file = Some("/var/run/mcp/../../other-secret".into());
        assert!(!valid_tool(&tool));
        tool.bearer_file = None;
        tool.input_schema["properties"]["project"]["enum"] = json!("frozen-project");
        assert!(
            !valid_tool(&tool),
            "malformed enum cannot silently widen project scope"
        );
        tool.input_schema["properties"]["project"]["enum"] = json!([]);
        assert!(!valid_tool(&tool));
        tool.input_schema["properties"]["project"]["enum"] = json!([3]);
        assert!(!valid_tool(&tool));
        tool.input_schema["properties"]["project"]["enum"] = json!(["frozen-project"]);
        tool.input_schema["properties"]["project"]["pattern"] = json!("ignored-pattern");
        assert!(
            !valid_tool(&tool),
            "unsupported constraints must not be ignored"
        );
    }

    #[test]
    fn endpoint_queries_cannot_enter_retained_tool_descriptors() {
        let mut tool = tool();
        for endpoint in [
            "https://example.invalid/mcp?access_token=public-test-value",
            "https://example.invalid/mcp?route=project",
            "https://example.invalid/mcp?",
            "http://127.0.0.1:8080/mcp?access_token=public-test-value",
        ] {
            tool.endpoint = endpoint.into();
            assert!(
                !valid_tool(&tool),
                "MCP query strings must be rejected before retention"
            );
        }
        tool.endpoint = "https://example.invalid/mcp".into();
        assert!(valid_tool(&tool));
    }

    #[tokio::test]
    async fn http_profile_initializes_scoped_tool_and_returns_only_bounded_result()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
        async fn handler(Json(body): Json<Value>) -> axum::response::Response {
            match body["method"].as_str() {
                Some("initialize") => Json(json!({"jsonrpc":"2.0","id":"initialize","result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}}}})).into_response(),
                Some("notifications/initialized") => StatusCode::ACCEPTED.into_response(),
                Some("tools/list") => Json(json!({"jsonrpc":"2.0","id":"tools","result":{"tools":[{"name":"project_lookup","inputSchema":{"type":"object","properties":{"project":{"type":"string","enum":["frozen-project"]}},"required":["project"],"additionalProperties":false}}]}})).into_response(),
                Some("tools/call") if body["params"]["arguments"]["project"] == "frozen-project" => Json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":[{"type":"text","text":"accepted fact"}]}})).into_response(),
                _ => StatusCode::BAD_REQUEST.into_response(),
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/mcp", listener.local_addr()?);
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/mcp", post(handler))).await
        });
        let mut tool = tool();
        tool.endpoint = endpoint;
        let result = invoke(&tool, "invocation_1", json!({"project":"frozen-project"}))
            .await
            .map_err(std::io::Error::other)?;
        assert!(result.contains("accepted fact"));
        tool.max_output_bytes = 8;
        assert_eq!(
            invoke(&tool, "invocation_2", json!({"project":"frozen-project"})).await,
            Err("provider_tool_output_limit")
        );
        server.abort();
        Ok(())
    }
}
