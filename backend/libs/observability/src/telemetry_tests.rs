use super::*;
use opentelemetry_sdk::trace::InMemorySpanExporter;
use tracing_subscriber::{Layer, prelude::*};

#[test]
fn verification_export_preserves_context_and_excludes_unrelated_sensitive_events()
-> Result<(), Box<dyn std::error::Error>> {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry().with(
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer("test"))
            .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                m.target() == TRACE_TARGET
            })),
    );
    let parent = TraceContext::parse(
        "00-7431fc7927bb4f208986f2a4f2418dba-3f49c016c2df402b-01",
        None,
    )
    .ok_or("context")?;
    let (bytes, original) = tracing::subscriber::with_default(subscriber, || {
        let span = verification_span("worker", "verify-test", Some(&parent));
        span.in_scope(|| {
            tracing::info!(target:"ayni_orchestrator_worker", error="private repository output");
            let raw = br#"{"id":"unchanged","data":{"binding":"unchanged"}}"#.to_vec();
            let bytes = with_event_context(raw.clone(), 16384);
            assert_eq!(with_event_context(raw.clone(), raw.len()), raw);
            (bytes, current_context())
        })
    });
    let context = event_context(&bytes).ok_or("event context")?;
    assert_eq!(Some(&context), original.as_ref());
    assert_eq!(&context.traceparent[3..35], &parent.traceparent[3..35]);
    let event: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(event["id"], "unchanged");
    provider.force_flush()?;
    let spans = exporter.get_finished_spans()?;
    assert_eq!(spans.len(), 1);
    assert!(spans[0].events.is_empty());
    assert!(!format!("{spans:?}").contains("private repository output"));
    provider.shutdown()?;
    Ok(())
}

#[test]
fn disabled_export_keeps_application_envelope_unchanged() {
    tracing::subscriber::with_default(tracing_subscriber::registry(), || {
        let raw = br#"{"id":"stable"}"#.to_vec();
        assert!(current_context().is_none());
        assert_eq!(with_event_context(raw.clone(), 16384), raw);
    });
}
