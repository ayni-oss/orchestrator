use super::*;

fn identity() -> EventIdentity {
    EventIdentity {
        source: "ayni://orchestrator/supervisors/workflow-application".into(),
        id: uuid::Uuid::now_v7().to_string(),
    }
}

fn metadata() -> WorkflowEventMetadata {
    WorkflowEventMetadata {
        produced_at: "2026-10-20T08:00:00+00:00".into(),
        traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into(),
    }
}

fn executor_identity(executor_instance_id: &str) -> EventIdentity {
    EventIdentity {
        source: format!("ayni://orchestrator/supervisors/{executor_instance_id}"),
        id: uuid::Uuid::now_v7().to_string(),
    }
}

fn binding(event_id: &str) -> ConversationTurnBinding {
    ConversationTurnBinding {
        workspace_id: format!("workspace:{}", uuid::Uuid::now_v7()),
        project_id: format!("project:{}", uuid::Uuid::now_v7()),
        conversation_id: format!("conversation:{}", uuid::Uuid::now_v7()),
        turn_id: format!("turn:{}", uuid::Uuid::now_v7()),
        assignment_id: uuid::Uuid::now_v7().to_string(),
        executor_generation: 1,
        execution_binding_revision: "scripted-v1".into(),
        context_revision: 1,
        command_event_id: event_id.into(),
        command_digest: String::new(),
    }
}

#[test]
fn command_round_trip_preserves_identity_and_scope() -> Result<(), ContractError> {
    let identity = identity();
    let payload = ConversationTurnRequested {
        binding: Some(binding(&identity.id)),
        input_message_id: format!("message:{}", uuid::Uuid::now_v7()),
        input_content: "hello".into(),
        context: vec![],
        ..Default::default()
    };
    let metadata = metadata();
    let bytes = encode_command(&identity, &metadata, &payload)?;
    assert_eq!(decode_command(&bytes)?, (identity, metadata, payload));
    let mut changed_extension: serde_json::Value = serde_json::from_slice(&bytes)?;
    changed_extension["workspaceid"] =
        serde_json::json!(format!("workspace:{}", uuid::Uuid::now_v7()));
    assert!(decode_command(&serde_json::to_vec(&changed_extension)?).is_err());
    let mut missing_time: serde_json::Value = serde_json::from_slice(&bytes)?;
    missing_time
        .as_object_mut()
        .ok_or(ContractError::Invalid("command JSON object"))?
        .remove("time");
    assert!(decode_command(&serde_json::to_vec(&missing_time)?).is_err());
    let mut invalid_trace: serde_json::Value = serde_json::from_slice(&bytes)?;
    invalid_trace["traceparent"] = serde_json::json!("unscoped-trace");
    assert!(decode_command(&serde_json::to_vec(&invalid_trace)?).is_err());
    Ok(())
}

#[test]
fn report_signature_bytes_are_stable_and_signature_is_excluded() -> Result<(), ContractError> {
    let executor_instance_id = uuid::Uuid::now_v7().to_string();
    let identity = executor_identity(&executor_instance_id);
    let mut report_binding = binding(&uuid::Uuid::now_v7().to_string());
    report_binding.command_digest = format!("sha256:{}", "a".repeat(64));
    let signing_seed = [7; 32];
    let public_key = SigningKey::from_bytes(&signing_seed)
        .verifying_key()
        .to_bytes();
    let mut payload = ConversationTurnReport {
        binding: Some(report_binding),
        kind: ConversationTurnReportKind::Started.into(),
        report_sequence: 1,
        executor_instance_id,
        signing_public_key: public_key.to_vec(),
        signing_key_digest: signing_key_digest(&public_key),
        signature: vec![],
        ..Default::default()
    };
    let expected = report_signing_bytes(&identity, &payload)?;
    sign_report(&identity, &mut payload, &signing_seed)?;
    verify_report_signature(&identity, &payload, None)?;
    let metadata = metadata();
    let bytes = encode_report(&identity, &metadata, &payload)?;
    let (actual_identity, actual_metadata, decoded) = decode_report(&bytes)?;
    assert_eq!(actual_identity, identity);
    assert_eq!(actual_metadata, metadata);
    assert_eq!(report_signing_bytes(&identity, &decoded)?, expected);
    let mut oversized_claim = bytes;
    assert!(oversized_claim.len() < MAX_CLAIM_BYTES);
    oversized_claim.resize(MAX_CLAIM_BYTES + 1, b' ');
    assert!(matches!(
        decode_report(&oversized_claim),
        Err(ContractError::Oversized)
    ));
    Ok(())
}

#[test]
fn tampered_report_signature_is_rejected() -> Result<(), ContractError> {
    let executor_instance_id = uuid::Uuid::now_v7().to_string();
    let identity = executor_identity(&executor_instance_id);
    let signing_seed = [11; 32];
    let public_key = SigningKey::from_bytes(&signing_seed)
        .verifying_key()
        .to_bytes();
    let mut report_binding = binding(&uuid::Uuid::now_v7().to_string());
    report_binding.command_digest = format!("sha256:{}", "d".repeat(64));
    let mut payload = ConversationTurnReport {
        binding: Some(report_binding),
        kind: ConversationTurnReportKind::Started.into(),
        report_sequence: 1,
        executor_instance_id,
        signing_public_key: public_key.to_vec(),
        signing_key_digest: signing_key_digest(&public_key),
        ..Default::default()
    };
    sign_report(&identity, &mut payload, &signing_seed)?;
    payload.executor_instance_id.push_str("-tampered");
    assert!(verify_report_signature(&identity, &payload, None).is_err());
    Ok(())
}

#[test]
fn weak_report_public_key_is_rejected() -> Result<(), ContractError> {
    let mut weak_public_key = [0; 32];
    weak_public_key[0] = 1;
    let parsed = VerifyingKey::from_bytes(&weak_public_key)
        .map_err(|_| ContractError::Invalid("weak-key test fixture"))?;
    assert!(parsed.is_weak());
    assert!(strict_verifying_key(&weak_public_key).is_err());
    Ok(())
}

#[test]
fn shared_signing_fixture_matches_canonical_bytes() -> Result<(), Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fixture {
        identity: EventIdentity,
        payload: ConversationTurnReport,
        canonical_hex: String,
        digest_hex: String,
        public_key_hex: String,
    }

    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../../../proto/testdata/workflow/report-signing.json"
    ))?;
    assert_eq!(
        hexadecimal(&report_signing_bytes(&fixture.identity, &fixture.payload)?),
        fixture.canonical_hex
    );
    assert_eq!(
        hexadecimal(&report_signing_digest(&fixture.identity, &fixture.payload)?),
        fixture.digest_hex
    );
    let public_key: [u8; 32] = decode_hexadecimal(&fixture.public_key_hex)?
        .try_into()
        .map_err(|_| "invalid public key length")?;
    verify_report_signature(&fixture.identity, &fixture.payload, Some(&public_key))?;
    Ok(())
}

fn hexadecimal(bytes: &[u8]) -> String {
    lowercase_hexadecimal(bytes)
}

fn decode_hexadecimal(value: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if !value.len().is_multiple_of(2) {
        return Err("odd hexadecimal length".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| Ok(u8::from_str_radix(&value[index..index + 2], 16)?))
        .collect()
}

#[test]
fn safe_reason_allowlists_reject_free_form_diagnostics() {
    assert!(report_failure_reason("script_failed"));
    assert!(!report_failure_reason("token=secret"));
    assert!(!report_failure_reason("raw_exception_text"));
    assert!(!report_failure_reason(&"a".repeat(64)));
    for reason in [
        "claim_lost",
        "stale_generation",
        "wrong_scope",
        "wrong_binding",
        "invalid_signature",
        "event_conflict",
        "terminal_conflict",
    ] {
        assert!(decision_reason_valid(
            ConversationReportDecisionKind::Rejected,
            reason
        ));
    }
    assert!(!decision_reason_valid(
        ConversationReportDecisionKind::Rejected,
        "credential_leaked_in_raw_diagnostic"
    ));
    assert!(decision_reason_valid(
        ConversationReportDecisionKind::Accepted,
        ""
    ));
    assert!(!decision_reason_valid(
        ConversationReportDecisionKind::Accepted,
        "accepted"
    ));
}

#[test]
fn decisions_are_targeted_and_bounded() -> Result<(), ContractError> {
    let identity = identity();
    let target_executor_instance_id = uuid::Uuid::now_v7().to_string();
    let payload = ConversationReportDecision {
        report_source: format!("ayni://orchestrator/supervisors/{target_executor_instance_id}"),
        report_id: uuid::Uuid::now_v7().to_string(),
        report_digest: format!("sha256:{}", "c".repeat(64)),
        target_executor_instance_id: target_executor_instance_id.clone(),
        accepted_executor_instance_id: target_executor_instance_id,
        assignment_id: uuid::Uuid::now_v7().to_string(),
        executor_generation: 1,
        decision: ConversationReportDecisionKind::Accepted.into(),
        safe_reason: String::new(),
        turn_state: "processing".into(),
        workspace_id: format!("workspace:{}", uuid::Uuid::now_v7()),
        project_id: format!("project:{}", uuid::Uuid::now_v7()),
        conversation_id: format!("conversation:{}", uuid::Uuid::now_v7()),
        turn_id: format!("turn:{}", uuid::Uuid::now_v7()),
    };
    let metadata = metadata();
    let bytes = encode_decision(&identity, &metadata, &payload)?;
    assert_eq!(
        decode_decision(&bytes)?,
        (identity.clone(), metadata.clone(), payload.clone())
    );

    let mut rejected = payload;
    rejected.decision = ConversationReportDecisionKind::Rejected.into();
    rejected.accepted_executor_instance_id.clear();
    rejected.safe_reason = "invalid_signature".into();
    rejected.turn_state = "queued".into();
    let bytes = encode_decision(&identity, &metadata, &rejected)?;
    assert_eq!(decode_decision(&bytes)?, (identity, metadata, rejected));
    Ok(())
}

#[test]
fn execution_inputs_and_session_scope_are_bounded_and_bound_to_the_command()
-> Result<(), ContractError> {
    use crate::v1::{
        AgentExecutionConfiguration, ConversationContextManifest, SessionExecutionScope,
    };
    let identity = identity();
    let binding = binding(&identity.id);
    let mut payload = ConversationTurnRequested {
        binding: Some(binding.clone()),
        input_message_id: format!("message:{}", uuid::Uuid::now_v7()),
        input_content: "hello".into(),
        context: Vec::new(),
        frozen_configuration: Some(AgentExecutionConfiguration {
            revision_id: uuid::Uuid::now_v7().to_string(),
            instructions: "Workspace instructions".into(),
            provider: "openai".into(),
            model: "explicit-model".into(),
            max_input_tokens: 8192,
            max_output_tokens: 2048,
            max_total_tokens: 10240,
            idle_timeout_seconds: 300,
        }),
        session: Some(SessionExecutionScope {
            session_id: uuid::Uuid::now_v7().to_string(),
            generation: binding.executor_generation,
            idle_timeout_seconds: 300,
        }),
        context_manifest: Some(ConversationContextManifest {
            id: binding.turn_id,
            digest: format!("sha256:{}", "a".repeat(64)),
            record_count: 1,
            inline_complete: false,
        }),
    };
    let metadata = metadata();
    let bytes = encode_command(&identity, &metadata, &payload)?;
    assert_eq!(decode_command(&bytes)?.2, payload);
    payload
        .session
        .as_mut()
        .ok_or(ContractError::Invalid("session fixture"))?
        .generation += 1;
    assert!(encode_command(&identity, &metadata, &payload).is_err());
    payload
        .session
        .as_mut()
        .ok_or(ContractError::Invalid("session fixture"))?
        .generation -= 1;
    payload
        .context_manifest
        .as_mut()
        .ok_or(ContractError::Invalid("manifest fixture"))?
        .inline_complete = true;
    assert!(encode_command(&identity, &metadata, &payload).is_err());
    payload
        .context_manifest
        .as_mut()
        .ok_or(ContractError::Invalid("manifest fixture"))?
        .inline_complete = false;
    payload
        .frozen_configuration
        .as_mut()
        .ok_or(ContractError::Invalid("config fixture"))?
        .instructions = "a".repeat(MAX_CONTENT_BYTES + 1);
    assert!(encode_command(&identity, &metadata, &payload).is_err());
    Ok(())
}
