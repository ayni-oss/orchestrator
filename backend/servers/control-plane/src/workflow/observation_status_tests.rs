use super::errors::RichStatus;
use super::*;

#[test]
fn accepted_receipt_handoff_meets_owned_latency_bound() {
    let context = RequestContext::new(&Request::new(()));
    let mut samples = Vec::new();
    for index in 0..105 {
        // Bounded persistence fake yields an already-committed receipt. The
        // owned adapter converts it and hands the response to Tonic directly.
        let accepted = model::CommandReceipt {
            command_id: "bounded-command".into(),
            kind: ReceiptKind::SendMessage,
            workspace_id: "bounded-workspace".into(),
            project_id: "bounded-project".into(),
            conversation_id: "bounded-conversation".into(),
            message_id: "bounded-message".into(),
            turn_id: "bounded-turn".into(),
            accepted_at: model::Timestamp {
                seconds: 0,
                nanos: 0,
            },
            conversation_revision: 1,
            resume_cursor: "bounded-cursor".into(),
            presentation_state: PresentationState::Accepted,
        };
        let committed = std::time::Instant::now();
        let response = context.response(v1::SendMessageResponse {
            receipt: Some(receipt(accepted)),
        });
        assert_eq!(
            response
                .get_ref()
                .receipt
                .as_ref()
                .map(|value| value.conversation_revision),
            Some(1)
        );
        if index >= 5 {
            samples.push(committed.elapsed());
        }
    }
    samples.sort();
    assert_eq!(samples.len(), 100);
    assert!(samples[94] <= std::time::Duration::from_millis(50));
    println!(
        "owned-receipt-handoff: samples=100 p95-us={} max-us={}",
        samples[94].as_micros(),
        samples[99].as_micros()
    );
}

#[test]
fn cursor_expiry_and_backpressure_keep_machine_readable_reasons() {
    let context = RequestContext::new(&Request::new(()));
    for (error, code, reason) in [
        (
            StoreError::WatchCursorExpired,
            Code::FailedPrecondition,
            v1::ErrorReason::WatchCursorExpired,
        ),
        (
            StoreError::Capacity("watch_buffer"),
            Code::ResourceExhausted,
            v1::ErrorReason::BoundExceeded,
        ),
    ] {
        let status = context.store(&error);
        assert_eq!(status.code(), code);
        let rich = RichStatus::decode(status.details()).unwrap_or_default();
        let details = rich
            .details
            .first()
            .and_then(|detail| v1::ErrorDetail::decode(detail.value.as_slice()).ok());
        assert_eq!(
            details.as_ref().map(|detail| detail.reason),
            Some(reason as i32)
        );
        assert_eq!(
            details.and_then(|detail| detail.request_id),
            Some(context.request_id.clone())
        );
    }
}
