//! One bounded coding pass. The provider can propose file contents, never commands.
use crate::responses::{PreparedRequest, ResponseError, ResponseEvidence};
use serde_json::{Value, json};
/// Prepare one complete bounded file-proposal request.
/// # Errors
/// Rejects invalid models or inputs outside the frozen budget.
pub fn prepare(
    model: &str,
    instructions: &str,
    task: &Value,
    max_input: u64,
    max_output: u64,
    max_total: u64,
) -> Result<PreparedRequest, ResponseError> {
    if model.is_empty() || model.len() > 128 {
        return Err(ResponseError::Configuration);
    }
    let tools = json!([{"type":"function","name":"propose_file_changes","description":"Return complete UTF-8 contents for at most 16 changed files. No shell, deletion, symlink, lock or policy changes.","strict":true,"parameters":{"type":"object","properties":{"changes":{"type":"array","maxItems":16,"items":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}}},"required":["changes"],"additionalProperties":false}},
    {"type":"function","name":"request_task_recovery","description":"Ask a blocking question and propose a bounded continuation for explicit user review. This ends this coding segment and cannot authorize further work.","strict":true,"parameters":{"type":"object","properties":{"question":{"type":"string"},"explanation":{"type":"string"},"action":{"type":"string","enum":["continue","repair_quality","replace_attempt"]},"additional_seconds":{"type":"integer","minimum":1,"maximum":86_400},"additional_tokens":{"type":"integer","minimum":1,"maximum":1_000_000}},"required":["question","explanation","action","additional_seconds","additional_tokens"],"additionalProperties":false}}]);
    let input = json!([{"role":"user","content":serde_json::to_string(task).map_err(|_|ResponseError::Configuration)?}]);
    let input_bound = u64::try_from(
        serde_json::to_vec(&json!({"instructions":instructions,"input":input,"tools":tools}))
            .map_err(|_| ResponseError::Configuration)?
            .len(),
    )
    .map_err(|_| ResponseError::InputLimit)?
    .saturating_add(1024);
    let output = max_output.min(
        max_total
            .checked_sub(input_bound)
            .ok_or(ResponseError::InputLimit)?,
    );
    if input_bound > max_input || output < 16 {
        return Err(ResponseError::InputLimit);
    }
    let bytes=serde_json::to_vec(&json!({"model":model,"instructions":instructions,"input":input,"tools":tools,"tool_choice":"required","parallel_tool_calls":false,"max_output_tokens":output,"store":false,"background":false,"stream":false,"truncation":"disabled"})).map_err(|_|ResponseError::Configuration)?;
    Ok(PreparedRequest {
        bytes,
        reserved_input_tokens: input_bound,
        reserved_output_tokens: output,
    })
}
/// Decode only the one permitted terminal file-proposal tool.
/// # Errors
/// Rejects provider failures, extra tools or oversized malformed output.
pub fn changes(evidence: &ResponseEvidence) -> Result<Value, &'static str> {
    if !evidence.failure_reason.is_empty()
        || evidence.tool_calls.len() != 1
        || evidence.tool_calls[0].name != "propose_file_changes"
        || evidence.tool_calls[0].arguments.len() > 64 * 1024
    {
        return Err("provider_output_invalid");
    }
    let value: Value = serde_json::from_str(&evidence.tool_calls[0].arguments)
        .map_err(|_| "provider_output_invalid")?;
    if value.as_object().is_none_or(|v| v.len() != 1) || !value["changes"].is_array() {
        return Err("provider_output_invalid");
    }
    Ok(value["changes"].clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coding_requests_bound_file_and_recovery_proposals_without_execution_authority() {
        let task = json!({"objective":"Change one file","files":[]});
        assert!(prepare("model", "", &task, 1, 100, 1000).is_err());
        let request = prepare("model", "frozen instructions", &task, 10000, 1000, 11000)
            .unwrap_or_else(|_| panic!("fixture"));
        let body: Value =
            serde_json::from_slice(&request.bytes).unwrap_or_else(|_| panic!("fixture"));
        assert_eq!(body["tools"].as_array().map(Vec::len), Some(2));
        assert_eq!(body["tools"][0]["name"], "propose_file_changes");
        assert_eq!(body["store"], false);
    }
}
