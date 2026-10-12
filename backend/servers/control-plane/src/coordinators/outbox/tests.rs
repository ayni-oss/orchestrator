use super::*;

#[tokio::test]
async fn transport_requires_explicit_local_opt_in_and_never_accepts_credentials()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let state = AppState::open(directory.path().join("state.sqlite"), 3)?;
    for endpoint in [
        "nats://user:password@localhost:4222",
        "tls://localhost:4222",
        "nats://localhost:4222?token=secret",
        "nats://localhost:4222/path",
        "nats://localhost:0",
    ] {
        assert!(OutboxPublisher::from_values(state.clone(), Some(endpoint), Some("true")).is_err());
    }
    assert!(
        OutboxPublisher::from_values(state.clone(), Some("nats://localhost:4222"), None).is_err()
    );
    assert!(matches!(
        OutboxPublisher::from_values(state.clone(), None, None),
        Ok(None)
    ));
    assert!(matches!(
        OutboxPublisher::from_values(state.clone(), Some("nats://jetstream:4222"), Some("true")),
        Ok(Some(_))
    ));
    state.shutdown_database().await?;
    Ok(())
}

#[test]
fn message_identity_is_stable_and_includes_unambiguous_source() {
    use ayni_orchestrator_events::EventIdentity;
    let first = EventIdentity {
        source: "ab".into(),
        id: "c".into(),
    };
    let other = EventIdentity {
        source: "a".into(),
        id: "bc".into(),
    };
    assert_eq!(message_id(&first), message_id(&first.clone()));
    assert_ne!(message_id(&first), message_id(&other));
    assert_eq!(message_id(&first).len(), 64);
    // Fixed-width big-endian prefix keeps the identity stable on every architecture.
    let expected = Sha256::digest([&2_u64.to_be_bytes()[..], b"abc"].concat());
    assert_eq!(message_id(&first), format!("{expected:x}"));
}
