//! Bounded diagnostic evidence; never a workflow outcome or provider qualification.
use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
const CONVERSATION: &[&str] = &[
    "api.message.admit",
    "outbox.publish",
    "supervisor.command.receive",
    "supervisor.session.execute",
    "session.pod.prepare",
    "session.pod.ready",
    "broker.report.accept",
];
const TASK: &[&str] = &[
    "task.api.start",
    "task.supervisor.reconcile",
    "task.source.prepare",
    "task.coding.execute",
    "task.coding.accept",
    "task.quality.freeze",
    "task.quality.collect",
    "task.quality.accept",
    "task.api.publish",
    "task.publication.reconcile",
    "task.publication.accept",
];
const REPLAY: &[&str] = &[
    "outbox.publish",
    "supervisor.command.receive",
    "broker.report.accept",
    "worker.scripted.execute",
    "task.coding.execute",
    "task.publication.reconcile",
    "provider.model.execute",
    "provider.tool.execute",
];
const FORBIDDEN: &[&str] = &[
    "prompt",
    "api_key",
    "authorization",
    "content",
    "tool_arguments",
    "tool_output",
    "raw_error",
    "password",
    "secret",
    "body",
    "exception.message",
    "exception.stacktrace",
];
const FIELDS: &[&str] = &[
    "stage",
    "workspace_id",
    "turn_id",
    "event_id",
    "provider_call_id",
    "tool_invocation_id",
    "parent_attempt_id",
    "replay",
    "project_id",
    "conversation_id",
    "message_id",
    "run_id",
    "plan_id",
    "plan_version",
    "proposal_id",
    "task_id",
    "attempt_id",
    "session_id",
    "generation",
    "model",
    "profile_revision",
    "outcome",
    "tool",
    "model_calls",
    "input_tokens",
    "output_tokens",
    "reason_code",
    "execution",
    "quality",
    "publication",
];
fn reject(value: &Value) -> Result<()> {
    match value {
        Value::Object(object) => {
            let key = object
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            ensure!(
                !FORBIDDEN.contains(&key.as_str())
                    && !key.split('.').any(|part| FORBIDDEN.contains(&part)),
                "content-bearing attribute in workflow traces"
            );
            for value in object.values() {
                reject(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn attributes(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some((
                item["key"].as_str()?.into(),
                item["value"].as_object()?.values().next()?.clone(),
            ))
        })
        .collect()
}
fn identity(value: &Value, size: usize) -> String {
    let value = value.as_str().unwrap_or_default();
    if value.len() == size * 2 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return value.to_lowercase();
    }
    STANDARD
        .decode(value)
        .ok()
        .filter(|bytes| bytes.len() == size)
        .map(|bytes| {
            bytes.iter().fold(String::new(), |mut text, byte| {
                let _ = write!(text, "{byte:02x}");
                text
            })
        })
        .unwrap_or_default()
}
fn projection(attributes: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    attributes
        .into_iter()
        .filter(|(key, value)| {
            FIELDS.contains(&key.as_str())
                && (value.is_boolean()
                    || value.is_i64()
                    || value.is_u64()
                    || value.as_str().is_some_and(|v| {
                        v == "[invalid]"
                            || (v.len() <= 128
                                && v.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"_.:/@-".contains(&b)))
                    }))
        })
        .collect()
}
fn spans(raw: &[u8]) -> Result<Vec<Value>> {
    ensure!(
        raw.len() <= 32 * 1024 * 1024,
        "trace evidence exceeds 32 MiB"
    );
    let mut result = Vec::new();
    for line in std::str::from_utf8(raw)?
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let value: Value = serde_json::from_str(line)?;
        reject(&value)?;
        for resource in value["resourceSpans"].as_array().into_iter().flatten() {
            let attrs = attributes(&resource["resource"]["attributes"]);
            let service = attrs
                .get("service.name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let service = if [
                "control-plane",
                "conversation-supervisor",
                "conversation-worker",
                "task-supervisor",
                "task-worker",
                "task-publisher",
                "workflow-migrate",
            ]
            .contains(&service)
            {
                service
            } else {
                "[other]"
            };
            for scope in resource["scopeSpans"].as_array().into_iter().flatten() {
                for span in scope["spans"].as_array().into_iter().flatten() {
                    let fields = projection(attributes(&span["attributes"]));
                    if !fields.contains_key("stage") {
                        continue;
                    }
                    let trace = identity(&span["traceId"], 16);
                    let id = identity(&span["spanId"], 8);
                    ensure!(
                        !trace.is_empty() && !id.is_empty(),
                        "invalid correlated span identity"
                    );
                    let links: Vec<_> = span["links"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|link| {
                            let trace = identity(&link["traceId"], 16);
                            let id = identity(&link["spanId"], 8);
                            (!trace.is_empty() && !id.is_empty())
                                .then(|| json!({"trace_id":trace,"span_id":id}))
                        })
                        .collect();
                    result.push(json!({"trace_id":trace,"span_id":id,"parent_span_id":identity(&span["parentSpanId"],8),"service":service,"fields":fields,"links":links}));
                }
            }
        }
    }
    Ok(result)
}
fn inspect(spans: &[Value], full: bool) -> Value {
    let mut groups = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut repeated = BTreeMap::new();
    for span in spans {
        let trace = span["trace_id"].as_str().unwrap_or_default();
        let stage = span["fields"]["stage"].as_str().unwrap_or_default();
        groups.entry(trace).or_default().insert(stage);
        *repeated.entry((trace, stage)).or_insert(0) += 1;
    }
    let conversations: Vec<_> = groups
        .iter()
        .filter(|(_, stages)| {
            CONVERSATION.iter().all(|s| stages.contains(s))
                && ["worker.scripted.execute", "provider.model.execute"]
                    .iter()
                    .any(|s| stages.contains(s))
        })
        .map(|(trace, _)| *trace)
        .collect();
    let tasks: Vec<_> = groups
        .iter()
        .filter(|(_, stages)| TASK.iter().all(|s| stages.contains(s)))
        .map(|(trace, _)| *trace)
        .collect();
    let replay = spans
        .iter()
        .filter(|s| {
            let stage = s["fields"]["stage"].as_str().unwrap_or_default();
            REPLAY.contains(&stage)
                && s["fields"]["replay"] == true
                && s["links"].as_array().is_some_and(|l| !l.is_empty())
                && repeated
                    .get(&(s["trace_id"].as_str().unwrap_or_default(), stage))
                    .is_some_and(|count| *count > 1)
        })
        .count();
    let interrupted = spans
        .iter()
        .filter(|s| s["fields"]["execution"] == "cancelling")
        .count();
    let mut gaps = Vec::new();
    if conversations.is_empty() {
        gaps.push("no single conversation trace spans admission through worker");
    }
    if full {
        if tasks.is_empty() {
            gaps.push("no single task trace spans coding, quality and publication");
        }
        if replay == 0 {
            gaps.push("no repeated retained operation with a valid causal link");
        }
        if interrupted == 0 {
            gaps.push("no cancellation reconciliation");
        }
    }
    json!({"passed":gaps.is_empty(),"span_count":spans.len(),"conversation_trace_ids":conversations,"task_attempt_trace_ids":tasks,"linked_repeated_operations":replay,"interrupted_operations":interrupted,"gaps":gaps,"authority":"diagnostics only; retained runtime acceptance proves outcomes"})
}
pub fn run(args: &[String]) -> Result<()> {
    let input = args
        .first()
        .context("traces INPUT_JSONL OUTPUT_DIRECTORY [conversation|full]")?;
    let output = args.get(1).context("output directory missing")?;
    let profile = args.get(2).map_or("full", String::as_str);
    ensure!(
        ["full", "conversation"].contains(&profile),
        "unknown trace profile"
    );
    ensure!(
        std::fs::metadata(input)?.len() <= 32 * 1024 * 1024,
        "trace evidence exceeds 32 MiB"
    );
    let spans = spans(&std::fs::read(input)?)?;
    let result = inspect(&spans, profile == "full");
    std::fs::create_dir(output)?;
    for (name, value) in [
        ("spans.json", json!(spans)),
        ("outcome.json", result.clone()),
    ] {
        std::fs::write(
            std::path::Path::new(output).join(name),
            serde_json::to_vec_pretty(&value)?,
        )?;
    }
    ensure!(result["passed"] == true, "trace gaps: {}", result["gaps"]);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_traces_and_polling_cannot_claim_a_journey() {
        let spans: Vec<_> = CONVERSATION
            .iter()
            .enumerate()
            .map(|(i, s)| json!({"trace_id":i.to_string(),"fields":{"stage":s}}))
            .collect();
        assert_eq!(inspect(&spans, false)["passed"], false);
        assert_eq!(inspect(&[], true)["passed"], false);
    }
    #[test]
    fn rejects_private_attributes_and_invalid_ids() -> Result<()> {
        assert!(reject(&json!({"events":[{"attributes":[{"key":"request.body"}]}]})).is_err());
        assert_eq!(identity(&json!("invalid"), 16), "");
        assert_eq!(
            identity(&json!(STANDARD.encode([7_u8; 16])), 16),
            "07".repeat(16)
        );
        assert!(
            projection(BTreeMap::from([
                ("stage".into(), json!("hello world")),
                ("unbounded".into(), json!("x"))
            ]))
            .is_empty()
        );
        assert!(spans(b"{\"resourceSpans\":[]}")?.is_empty());
        Ok(())
    }
}
