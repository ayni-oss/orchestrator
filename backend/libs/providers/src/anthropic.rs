//! Claude Messages adapter for the retained stateless exchange contract.
//! Thinking and server tools are deliberately unavailable: only approved client tools run.
use crate::responses::{FunctionCall, ResponseError, ResponseEvidence, TokenUsage};
use serde_json::{Value, json};

pub fn request(bytes: &[u8]) -> Result<Vec<u8>, ResponseError> {
    let source: Value = serde_json::from_slice(bytes).map_err(|_| ResponseError::Configuration)?;
    let mut messages: Vec<Value> = Vec::new();
    for item in source["input"]
        .as_array()
        .ok_or(ResponseError::Configuration)?
    {
        let (role, block) = match item["type"].as_str() {
            Some("function_call") => (
                "assistant",
                json!({"type":"tool_use","id":item["call_id"],"name":item["name"],"input":serde_json::from_str::<Value>(item["arguments"].as_str().ok_or(ResponseError::Configuration)?).map_err(|_|ResponseError::Configuration)?}),
            ),
            Some("function_call_output") => (
                "user",
                json!({"type":"tool_result","tool_use_id":item["call_id"],"content":item["output"]}),
            ),
            None | Some("message") => {
                let role = item["role"]
                    .as_str()
                    .filter(|r| matches!(*r, "user" | "assistant"))
                    .ok_or(ResponseError::Configuration)?;
                let text = item["content"]
                    .as_str()
                    .ok_or(ResponseError::Configuration)?;
                (role, json!({"type":"text","text":text}))
            }
            _ => return Err(ResponseError::Configuration),
        };
        if let Some(last) = messages.last_mut().filter(|last| last["role"] == role) {
            last["content"]
                .as_array_mut()
                .ok_or(ResponseError::Configuration)?
                .push(block);
        } else {
            messages.push(json!({"role":role,"content":[block]}));
        }
    }
    let tools=source["tools"].as_array().ok_or(ResponseError::Configuration)?.iter().map(|tool|json!({"name":tool["name"],"description":tool["description"],"input_schema":tool["parameters"]})).collect::<Vec<_>>();
    let mut body = json!({"model":source["model"],"system":source["instructions"],"messages":messages,"max_tokens":source["max_output_tokens"],"tools":tools,"stream":false,"thinking":{"type":"disabled"}});
    let choice = match source["tool_choice"].as_str() {
        Some("required") => Some("any"),
        Some("auto") => Some("auto"),
        None if source["parallel_tool_calls"] == false => Some("auto"),
        None => None,
        _ => return Err(ResponseError::Configuration),
    };
    if let Some(choice) = choice {
        let mut selected = json!({"type":choice});
        if source["parallel_tool_calls"] == false {
            selected["disable_parallel_tool_use"] = json!(true);
        }
        body["tool_choice"] = selected;
    }
    serde_json::to_vec(&body).map_err(|_| ResponseError::Configuration)
}

pub fn response(bytes: &[u8]) -> Result<ResponseEvidence, ResponseError> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| ResponseError::Uncertain)?;
    if body["type"] != "message" || body["role"] != "assistant" {
        return Err(ResponseError::Uncertain);
    }
    let field = |name: &str| {
        body[name]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .map(str::to_owned)
            .ok_or(ResponseError::Uncertain)
    };
    let (content, tool_calls) = response_blocks(&body)?;
    let failure_reason = match body["stop_reason"].as_str() {
        Some("end_turn") if tool_calls.is_empty() => "",
        Some("tool_use") if !tool_calls.is_empty() => "",
        Some("max_tokens") => "provider_output_limit",
        _ => "provider_rejected",
    };
    if content.len() > 16384
        || content
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ResponseError::Uncertain);
    }
    if content.is_empty() && tool_calls.is_empty() && failure_reason.is_empty() {
        return Err(ResponseError::Uncertain);
    }
    // Cache tokens are part of input usage. Missing fields stay unknown.
    let usage = &body["usage"];
    let input = usage["input_tokens"]
        .as_u64()
        .and_then(|n| n.checked_add(usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)))
        .and_then(|n| n.checked_add(usage["cache_read_input_tokens"].as_u64().unwrap_or(0)));
    let output = usage["output_tokens"].as_u64();
    Ok(ResponseEvidence {
        response_id: field("id")?,
        model: field("model")?,
        content: if failure_reason.is_empty() {
            content
        } else {
            String::new()
        },
        tool_calls: if failure_reason.is_empty() {
            tool_calls
        } else {
            Vec::new()
        },
        failure_reason: failure_reason.into(),
        usage: TokenUsage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input.and_then(|n| output.and_then(|o| n.checked_add(o))),
        },
    })
}

fn response_blocks(body: &Value) -> Result<(String, Vec<FunctionCall>), ResponseError> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for block in body["content"].as_array().ok_or(ResponseError::Uncertain)? {
        match block["type"].as_str() {
            Some("text") => {
                content.push_str(block["text"].as_str().ok_or(ResponseError::Uncertain)?);
            }
            Some("tool_use") => {
                let bounded = |name: &str| {
                    block[name]
                        .as_str()
                        .filter(|s| {
                            !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
                        })
                        .map(str::to_owned)
                        .ok_or(ResponseError::Uncertain)
                };
                if !block["input"].is_object() || tool_calls.len() >= 8 {
                    return Err(ResponseError::Uncertain);
                }
                let arguments =
                    serde_json::to_string(&block["input"]).map_err(|_| ResponseError::Uncertain)?;
                if arguments.len() > 40960 {
                    return Err(ResponseError::Uncertain);
                }
                tool_calls.push(FunctionCall {
                    call_id: bounded("id")?,
                    name: bounded("name")?,
                    arguments,
                });
            }
            _ => return Err(ResponseError::Uncertain),
        }
    }
    Ok((content, tool_calls))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "deterministic provider fixtures")]
mod tests {
    use super::*;
    #[test]
    fn stateless_tools_round_trip_without_provider_conversation_state() {
        let source = json!({"model":"explicit-model","instructions":"frozen","max_output_tokens":128,"tools":[{"name":"read_project_status","description":"facts","parameters":{"type":"object"}}],"input":[{"role":"user","content":"status"},{"type":"function_call","call_id":"call1","name":"read_project_status","arguments":"{}"},{"type":"function_call_output","call_id":"call1","output":"facts"}]});
        let body: Value =
            serde_json::from_slice(&request(&serde_json::to_vec(&source).unwrap()).unwrap())
                .unwrap();
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["thinking"]["type"], "disabled");
        let result=response(br#"{"type":"message","role":"assistant","id":"msg1","model":"model","stop_reason":"tool_use","content":[{"type":"tool_use","id":"call1","name":"read_project_status","input":{}}],"usage":{"input_tokens":4,"cache_read_input_tokens":2,"output_tokens":3}}"#).unwrap();
        assert_eq!(result.tool_calls[0].arguments, "{}");
        assert_eq!(result.usage.total_tokens, Some(9));
    }
    #[test]
    fn required_and_single_tool_execution_survive_provider_translation() {
        let task = json!({"objective":"one change","files":[]});
        let prepared =
            crate::coding::prepare("model", "frozen", &task, 10000, 1000, 11000).unwrap();
        let body: Value = serde_json::from_slice(&request(&prepared.bytes).unwrap()).unwrap();
        assert_eq!(
            body["tool_choice"],
            json!({"type":"any","disable_parallel_tool_use":true})
        );
        let mut normalized: Value = serde_json::from_slice(&prepared.bytes).unwrap();
        normalized.as_object_mut().unwrap().remove("tool_choice");
        let body: Value =
            serde_json::from_slice(&request(&serde_json::to_vec(&normalized).unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            body["tool_choice"],
            json!({"type":"auto","disable_parallel_tool_use":true})
        );
        normalized["tool_choice"] = json!("unsupported");
        assert!(request(&serde_json::to_vec(&normalized).unwrap()).is_err());
    }
    #[test]
    fn unknown_usage_stays_unknown_and_private_or_server_blocks_are_rejected() {
        let result=response(br#"{"type":"message","role":"assistant","id":"msg1","model":"model","stop_reason":"end_turn","content":[{"type":"text","text":"answer"}]}"#).unwrap();
        assert_eq!(result.usage.total_tokens, None);
        assert!(response(br#"{"type":"message","role":"assistant","id":"msg1","model":"model","stop_reason":"end_turn","content":[{"type":"thinking","thinking":"private"}]}"#).is_err());
    }
}
