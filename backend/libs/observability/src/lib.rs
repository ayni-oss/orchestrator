//! Shared, fail-closed structured logging for control-plane and worker processes.

use std::env;
use tracing_subscriber::{Layer, prelude::*};
mod telemetry;
pub mod workflow;
pub use telemetry::{
    TelemetryGuard, TraceContext, current_context, event_context, exporter_endpoint,
    record_failure, verification_span, with_event_context,
};
use tracing::{Span, field, info};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

const LOG_TARGETS: &[&str] = &[
    "ayni_orchestrator_control_plane",
    "ayni_orchestrator_observability",
    "ayni_orchestrator_worker",
];

const SENSITIVE_ENVIRONMENT_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "DEVELOPMENT_PUBLISH_TOKEN",
    "GITHUB_CLONE_TOKEN",
    "DEVELOPMENT_REQUEST",
    "DEVELOPMENT_ACCEPTANCE_CRITERIA",
    "DEVELOPMENT_CHANGE_CONSTRAINTS",
    "DEVELOPMENT_PR_CONTEXT",
    "ORCHESTRATOR_REPOSITORY_URL",
];

/// Initializes newline-delimited JSON logs on stdout.
pub fn init(service: &'static str) -> TelemetryGuard {
    let guard = telemetry::provider(service);
    let bridge = guard.tracer().map(|tracer| {
        tracing_opentelemetry::layer()
            .with_tracer(tracer)
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                matches!(
                    metadata.target(),
                    telemetry::TRACE_TARGET | workflow::WORKFLOW_TARGET
                )
            }))
    });
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .with_span_events(FmtSpan::CLOSE)
                .with_target(true)
                .with_writer(std::io::stdout)
                .with_filter(configured_filter()),
        )
        .with(bridge)
        .init();
    info!(service, log_format = "json", "logging initialized");
    guard
}

fn configured_level(value: Option<&str>) -> &str {
    value
        .filter(|value| matches!(*value, "error" | "warn" | "info" | "debug" | "trace"))
        .unwrap_or("info")
}

fn configured_filter() -> EnvFilter {
    let configured = env::var("RUST_LOG").ok();
    let level = configured_level(configured.as_deref());
    let directives = LOG_TARGETS
        .iter()
        .fold("off".to_owned(), |mut value, target| {
            value.push(',');
            value.push_str(target);
            value.push('=');
            value.push_str(level);
            value
        });
    EnvFilter::new(directives)
}

/// Creates a process span populated only from an allowlist of bounded correlation fields.
pub fn process_span(service: &'static str) -> Span {
    let span = tracing::info_span!(
        "process",
        service,
        task_id = field::Empty,
        attempt_id = field::Empty,
        preparation_id = field::Empty,
        stage = field::Empty,
        job_name = field::Empty,
        provider = field::Empty,
        model = field::Empty,
    );
    for (field, key) in [
        ("task_id", "DEVELOPMENT_TASK_ID"),
        ("attempt_id", "ORCHESTRATOR_ATTEMPT_ID"),
        ("preparation_id", "DEVELOPMENT_PREPARATION_ID"),
        ("stage", "DEVELOPMENT_STAGE"),
        ("job_name", "JOB_NAME"),
        ("provider", "AGENT_PROVIDER"),
        ("model", "AGENT_MODEL"),
    ] {
        if let Ok(value) = env::var(key) {
            span.record(field, bounded_correlation(&value));
        }
    }
    span
}

fn bounded_correlation(value: &str) -> &str {
    if !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':' | b'@')
        })
    {
        value
    } else {
        "[invalid]"
    }
}

/// Removes known credential and prompt values before an error is emitted.
///
/// Callers should still prefer stable reason codes when an upstream error can contain
/// repository content or full tool output.
#[must_use]
pub fn redact(message: &str) -> String {
    let sensitive = SENSITIVE_ENVIRONMENT_KEYS
        .iter()
        .filter_map(|key| env::var(key).ok())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    redact_values(message, &sensitive)
}

fn redact_values(message: &str, sensitive: &[String]) -> String {
    let mut redacted = message.to_owned();
    for value in sensitive {
        if !value.is_empty() {
            redacted = redacted.replace(value, "[REDACTED]");
        }
    }
    for marker in ["authorization:", "authorization=", "bearer "] {
        redacted = redact_after_marker(&redacted, marker);
    }
    redacted
}

fn redact_after_marker(value: &str, marker: &str) -> String {
    let mut output = value.to_owned();
    let mut search_from = 0;
    loop {
        let lowercase = output.to_ascii_lowercase();
        let Some(relative) = lowercase[search_from..].find(marker) else {
            break;
        };
        let start = search_from + relative + marker.len();
        let end = output[start..]
            .find(['\n', '\r', ',', ';'])
            .map_or(output.len(), |offset| start + offset);
        output.replace_range(start..end, "[REDACTED]");
        search_from = start + "[REDACTED]".len();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{SENSITIVE_ENVIRONMENT_KEYS, bounded_correlation, configured_level, redact_values};
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };
    use tracing::info;
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufferWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("log buffer lock poisoned"))?
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = BufferWriter;

        fn make_writer(&'a self) -> Self::Writer {
            BufferWriter(Arc::clone(&self.0))
        }
    }

    #[test]
    fn redaction_removes_credentials_prompts_and_authorization_headers() {
        let secret = "sk-private-value".to_owned();
        let prompt = "change proprietary formula".to_owned();
        let header_credential = ["header", "credential"].join("-");
        let value = format!(
            "provider failed for {prompt}; Authorization: Bearer {secret}\nretry Bearer {header_credential}"
        );
        let redacted = redact_values(&value, &[secret.clone(), prompt.clone()]);
        assert!(!redacted.contains(&secret));
        assert!(!redacted.contains(&prompt));
        assert!(!redacted.contains(&header_credential));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn emitted_lines_are_json_and_redacted() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_ansi(false)
            .with_writer(Buffer(Arc::clone(&bytes)))
            .finish();
        let credential = ["private", "credential"].join("-");
        let prompt = "proprietary request".to_owned();
        tracing::subscriber::with_default(subscriber, || {
            info!(
                error = %redact_values(
                    &format!("Authorization: Bearer {credential}; {prompt}"),
                    &[credential.clone(), prompt.clone()],
                ),
                "worker failed"
            );
        });
        let output = String::from_utf8(
            bytes
                .lock()
                .map_err(|_| io::Error::other("log buffer lock poisoned"))?
                .clone(),
        )?;
        assert!(!output.contains(&credential));
        assert!(!output.contains(&prompt));
        for line in output.lines() {
            let value: serde_json::Value = serde_json::from_str(line)?;
            assert_eq!(value["message"], "worker failed");
        }
        Ok(())
    }

    #[test]
    fn dependency_log_directives_cannot_enable_sensitive_targets() {
        assert_eq!(configured_level(Some("debug")), "debug");
        assert_eq!(configured_level(Some("reqwest=trace")), "info");
        assert_eq!(configured_level(Some("trace,hyper=trace")), "info");
        assert!(SENSITIVE_ENVIRONMENT_KEYS.contains(&"ORCHESTRATOR_REPOSITORY_URL"));
    }

    #[test]
    fn correlation_fields_are_bounded_and_single_line() {
        assert_eq!(bounded_correlation("task-123/model"), "task-123/model");
        assert_eq!(bounded_correlation("bad\nfield"), "[invalid]");
        assert_eq!(bounded_correlation(&"x".repeat(129)), "[invalid]");
    }
}
