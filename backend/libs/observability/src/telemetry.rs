//! Optional allowlisted verification and workflow export. Diagnostic context conveys no authority.
use opentelemetry::{
    Context, KeyValue,
    propagation::TextMapPropagator,
    trace::{TraceContextExt, TracerProvider},
};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub const TRACE_TARGET: &str = "ayni_orchestrator_observability::verification";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TraceContext {
    pub traceparent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}
impl TraceContext {
    /// Invalid diagnostic inputs are ignored rather than rejecting application work.
    #[must_use]
    pub fn parse(parent: &str, state: Option<&str>) -> Option<Self> {
        if parent.len() != 55 || !parent.is_ascii() {
            return None;
        }
        let mut carrier = HashMap::from([("traceparent".to_owned(), parent.to_owned())]);
        if let Some(state) =
            state.filter(|s| s.len() <= 512 && s.is_ascii() && !s.contains(['\r', '\n']))
        {
            carrier.insert("tracestate".into(), state.into());
        }
        let context = TraceContextPropagator::new().extract(&carrier);
        if !context.span().span_context().is_valid() {
            return None;
        }
        let mut normalized = HashMap::new();
        TraceContextPropagator::new().inject_context(&context, &mut normalized);
        Some(Self {
            traceparent: normalized.remove("traceparent")?,
            tracestate: normalized.remove("tracestate").filter(|s| !s.is_empty()),
        })
    }
    pub(crate) fn context(&self) -> Context {
        let mut carrier = HashMap::from([("traceparent".to_owned(), self.traceparent.clone())]);
        if let Some(state) = &self.tracestate {
            carrier.insert("tracestate".into(), state.clone());
        }
        TraceContextPropagator::new().extract(&carrier)
    }
}

#[must_use]
pub fn current_context() -> Option<TraceContext> {
    let context = tracing::Span::current().context();
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(&context, &mut carrier);
    TraceContext::parse(
        carrier.get("traceparent")?,
        carrier.get("tracestate").map(String::as_str),
    )
}

#[must_use]
pub fn verification_span(
    stage: &'static str,
    execution: &str,
    context: Option<&TraceContext>,
) -> tracing::Span {
    let span = tracing::info_span!(target: TRACE_TARGET, "verification", stage, execution_id = super::bounded_correlation(execution), reason_code = tracing::field::Empty,
        event_id = tracing::field::Empty,
        preparation_id = tracing::field::Empty, report_bytes = tracing::field::Empty);
    if let Some(context) = context {
        let parent = context.context();
        let _ = span.set_parent(parent.clone());
        if matches!(stage, "reconcile" | "cleanup" | "delivery") {
            span.add_link(parent.span().span_context().clone());
        }
    }
    span
}

/// Record only a bounded classification code, never an upstream error message.
pub fn record_failure(span: &tracing::Span, reason: &str) {
    let reason = super::bounded_correlation(reason);
    span.record("reason_code", reason);
    span.set_status(opentelemetry::trace::Status::error(reason.to_owned()));
}

/// Recover originating metadata from a bounded structured event. Invalid context is optional.
#[must_use]
pub fn event_context(bytes: &[u8]) -> Option<TraceContext> {
    let event: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    TraceContext::parse(
        event.get("traceparent")?.as_str()?,
        event.get("tracestate").and_then(serde_json::Value::as_str),
    )
}
/// Adding diagnostics must never make an otherwise valid application event oversized.
#[must_use]
pub fn with_event_context(bytes: Vec<u8>, max: usize) -> Vec<u8> {
    let Some(context) = current_context() else {
        return bytes;
    };
    let Ok(mut event) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return bytes;
    };
    let Some(object) = event.as_object_mut() else {
        return bytes;
    };
    object.insert("traceparent".into(), context.traceparent.into());
    if let Some(state) = context.tracestate {
        object.insert("tracestate".into(), state.into());
    }
    serde_json::to_vec(&event)
        .ok()
        .filter(|value| value.len() <= max)
        .unwrap_or(bytes)
}

/// Enable export only with an explicit, administrator-controlled trace endpoint.
#[must_use]
pub fn exporter_endpoint() -> Option<String> {
    let value = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").ok()?;
    let url = url::Url::parse(&value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(value)
}

pub struct TelemetryGuard(Option<SdkTracerProvider>);
impl TelemetryGuard {
    pub(super) fn tracer(&self) -> Option<opentelemetry_sdk::trace::SdkTracer> {
        self.0
            .as_ref()
            .map(|provider| provider.tracer("ayni-orchestrator"))
    }
    /// Call after operation spans close. Export failures never change the exit code.
    pub fn shutdown(self) {
        if let Some(provider) = self.0 {
            let _ = provider.shutdown_with_timeout(Duration::from_secs(3));
        }
    }
}
pub fn provider(service: &'static str) -> TelemetryGuard {
    let provider = exporter_endpoint().and_then(|endpoint| {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(endpoint)
            .with_timeout(Duration::from_secs(2))
            .build()
            .ok()?;
        let processor = BatchSpanProcessor::builder(exporter)
            .with_batch_config(
                BatchConfigBuilder::default()
                    .with_max_queue_size(512)
                    .with_max_export_batch_size(128)
                    .with_scheduled_delay(Duration::from_secs(1))
                    .build(),
            )
            .build();
        Some(
            SdkTracerProvider::builder()
                .with_span_processor(processor)
                .with_max_attributes_per_span(48)
                .with_max_events_per_span(8)
                .with_max_links_per_span(8)
                .with_resource(
                    Resource::builder_empty()
                        .with_attributes([KeyValue::new("service.name", service)])
                        .build(),
                )
                .build(),
        )
    });
    TelemetryGuard(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_context_is_ignored_and_valid_context_roundtrips() {
        for value in [
            "",
            "bad",
            "00-00000000000000000000000000000000-0000000000000000-01",
        ] {
            assert!(TraceContext::parse(value, None).is_none());
        }
        let value = "00-7431fc7927bb4f208986f2a4f2418dba-3f49c016c2df402b-01";
        let context = TraceContext::parse(value, Some("bad\nstate"));
        assert_eq!(
            context.as_ref().map(|c| c.traceparent.as_str()),
            Some(value)
        );
        assert!(context.is_some_and(|c| c.tracestate.is_none()));
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod export_tests;
