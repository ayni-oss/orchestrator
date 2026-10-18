//! Operator diagnostics retain public interaction text, never opaque reasoning state.
use rig_core::message::{AssistantContent, ReasoningContent};

pub(super) fn assistant_content(content: &[AssistantContent]) -> Vec<(bool, String)> {
    content
        .iter()
        .flat_map(|item| match item {
            AssistantContent::Text(text) => vec![(false, text.text.clone())],
            AssistantContent::Reasoning(reasoning) => reasoning
                .content
                .iter()
                .filter_map(|part| {
                    if let ReasoningContent::Summary(summary) = part {
                        Some((true, summary.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            _ => vec![],
        })
        .collect()
}

pub(super) fn redact(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets {
        text = text.replace(secret, "[REDACTED]");
    }
    for prefix in ["sk-", "ghp_", "ghs_", "github_pat_", "Bearer ", "bearer "] {
        let mut start = 0;
        while let Some(index) = text[start..].find(prefix) {
            let begin = start + index;
            let end = text[begin + prefix.len()..]
                .find(|c: char| c.is_whitespace() || matches!(c, '\"' | '\'' | ',' | '}' | ']'))
                .map_or(text.len(), |length| begin + prefix.len() + length);
            text.replace_range(begin..end, "[REDACTED]");
            start = begin + "[REDACTED]".len();
        }
    }
    text
}

pub(super) fn truncate(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub(super) fn prompt_diagnostic(error: &rig_agent::completion::PromptError) -> String {
    if let rig_agent::completion::PromptError::CompletionError(error) = error {
        let status = error
            .provider_response_status()
            .map_or_else(|| "unavailable".into(), |status| status.to_string());
        let body = error.provider_response_json().ok().flatten();
        let detail = body.as_ref().map(|body| body.get("error").unwrap_or(body));
        let fields = ["code", "type", "message"]
            .iter()
            .filter_map(|key| {
                detail
                    .and_then(|detail| detail.get(key))
                    .and_then(serde_json::Value::as_str)
                    .map(|value| format!("{key}: {value}"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Model request failed; HTTP status: {status}\n{fields}")
    } else {
        super::activity::prompt_failure(error).message().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::message::Reasoning;
    #[test]
    fn exposes_only_provider_summaries() {
        let content = vec![AssistantContent::Reasoning(Reasoning {
            id: None,
            content: vec![
                ReasoningContent::Encrypted("opaque".into()),
                ReasoningContent::Text {
                    text: "private".into(),
                    signature: None,
                },
                ReasoningContent::Redacted {
                    data: "hidden".into(),
                },
                ReasoningContent::Summary("Checking the lock contract".into()),
            ],
        })];
        assert_eq!(
            assistant_content(&content),
            vec![(true, "Checking the lock contract".into())]
        );
    }
    #[test]
    fn redacts_registered_credentials_and_common_token_forms() {
        let value = redact(
            "key=exact-secret token=sk-abc123\nAuthorization: Bearer abc123\nmessage",
            &["exact-secret".into()],
        );
        assert!(!value.contains("exact-secret"));
        assert!(!value.contains("abc123"));
        assert!(value.contains("message"));
        assert_eq!(truncate("ééé", 3), "é");
    }
}
