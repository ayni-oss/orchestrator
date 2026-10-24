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
        provider_call: None,
        plan_proposal: None,
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
        project_status: None,
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

#[test]
fn provider_usage_preserves_unknown_and_rejects_inconsistent_or_unbounded_counts() {
    use crate::v1::ProviderCallEvidence;
    let mut call = ProviderCallEvidence::default();
    assert!(provider_counters_valid(&call));
    assert_eq!(call.total_tokens, None);
    call.input_tokens = Some(0);
    call.output_tokens = Some(0);
    call.total_tokens = Some(0);
    assert!(provider_counters_valid(&call));
    call.input_tokens = Some(4);
    call.output_tokens = Some(2);
    call.total_tokens = Some(5);
    assert!(!provider_counters_valid(&call));
    call.output_tokens = None;
    call.total_tokens = Some(3);
    assert!(!provider_counters_valid(&call));
    call.input_tokens = Some(u64::MAX);
    call.total_tokens = None;
    assert!(!provider_counters_valid(&call));
}

#[test]
fn project_snapshot_cannot_expand_command_scope_or_claim_completion() -> Result<(), ContractError> {
    let id = identity();
    let binding = binding(&id.id);
    let mut payload = ConversationTurnRequested {
        binding: Some(binding.clone()),
        input_message_id: format!("message:{}", uuid::Uuid::now_v7()),
        input_content: "hello".into(),
        project_status: Some(crate::v1::ProjectStatusSnapshot {
            snapshot_id: id.id.clone(),
            project_id: binding.project_id,
            objective: "Factual goal".into(),
            status: "registered".into(),
            context_revision: binding.context_revision,
            project_resource_version: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(encode_command(&id, &metadata(), &payload).is_ok());
    payload
        .project_status
        .as_mut()
        .ok_or(ContractError::Invalid("snapshot"))?
        .project_id = format!("project:{}", uuid::Uuid::now_v7());
    assert!(encode_command(&id, &metadata(), &payload).is_err());
    payload
        .project_status
        .as_mut()
        .ok_or(ContractError::Invalid("snapshot"))?
        .project_id = payload
        .binding
        .as_ref()
        .ok_or(ContractError::Invalid("binding"))?
        .project_id
        .clone();
    payload
        .project_status
        .as_mut()
        .ok_or(ContractError::Invalid("snapshot"))?
        .status = "completed".into();
    assert!(encode_command(&id, &metadata(), &payload).is_err());
    Ok(())
}

#[test]
fn tool_projection_rejects_untrusted_names_duplicate_ids_and_unbounded_batches() {
    let tool = crate::v1::ToolInvocationEvidence {
        invocation_id: "invocation_1".into(),
        name: "read_project_status".into(),
        input_digest: format!("sha256:{}", "a".repeat(64)),
        output_digest: format!("sha256:{}", "b".repeat(64)),
        state: "completed".into(),
    };
    assert!(tool_invocations_valid(std::slice::from_ref(&tool)));
    assert!(!tool_invocations_valid(&[tool.clone(), tool.clone()]));
    let mut unknown = tool.clone();
    unknown.name = "shell".into();
    assert!(!tool_invocations_valid(&[unknown]));
    let mut untrusted = tool.clone();
    untrusted.invocation_id = "raw diagnostic\nsecret".into();
    assert!(!tool_invocations_valid(&[untrusted]));
    let tools = (0..9)
        .map(|index| crate::v1::ToolInvocationEvidence {
            invocation_id: format!("invocation_{index}"),
            ..tool.clone()
        })
        .collect::<Vec<_>>();
    assert!(!tool_invocations_valid(&tools));
}

#[test]
fn proposal_invocation_and_exact_bytes_are_covered_by_terminal_signature()
-> Result<(), ContractError> {
    let executor_id = uuid::Uuid::now_v7().to_string();
    let identity = executor_identity(&executor_id);
    let signing_seed = [19; 32];
    let key = SigningKey::from_bytes(&signing_seed)
        .verifying_key()
        .to_bytes();
    let mut binding = binding(&uuid::Uuid::now_v7().to_string());
    binding.command_digest = format!("sha256:{}", "a".repeat(64));
    binding.execution_binding_revision = "openai-responses-v1".into();
    let call = crate::v1::ProviderCallEvidence {
        call_id: binding.command_event_id.clone(),
        provider: "openai".into(),
        model: "explicit-model".into(),
        configuration_revision_id: uuid::Uuid::now_v7().to_string(),
        context_digest: format!("sha256:{}", "b".repeat(64)),
        request_digest: format!("sha256:{}", "c".repeat(64)),
        response_id: "response-identity".into(),
        reported_model: "explicit-model".into(),
        reserved_input_tokens: 8192,
        reserved_output_tokens: 4096,
        reserved_total_tokens: 12288,
        state: "completed".into(),
        ..Default::default()
    };
    let mut report = ConversationTurnReport {
        binding: Some(binding),
        kind: ConversationTurnReportKind::Completed.into(),
        report_sequence: 2,
        executor_instance_id: executor_id,
        signing_key_digest: signing_key_digest(&key),
        response_content: "Draft retained for review".into(),
        provider_call: Some(call),
        plan_proposal: Some(crate::v1::PlanProposal {
            invocation_id: "proposal_call".into(),
            proposal_json: vec![b'a'; 40960],
        }),
        ..Default::default()
    };
    sign_report(&identity, &mut report, &signing_seed)?;
    let bytes = encode_report(&identity, &metadata(), &report)?;
    assert!(bytes.len() > 32 * 1024);
    assert!(bytes.len() < MAX_REPORT_BYTES);
    let (_, _, retained) = decode_report(&bytes)?;
    verify_report_signature(&identity, &retained, Some(&key))?;
    let mut changed = retained.clone();
    changed
        .plan_proposal
        .as_mut()
        .ok_or(ContractError::Invalid("fixture proposal"))?
        .proposal_json[0] = b'b';
    assert!(verify_report_signature(&identity, &changed, Some(&key)).is_err());
    changed = retained;
    changed
        .plan_proposal
        .as_mut()
        .ok_or(ContractError::Invalid("fixture proposal"))?
        .invocation_id = "other".into();
    assert!(verify_report_signature(&identity, &changed, Some(&key)).is_err());
    Ok(())
}

#[test]
#[allow(
    clippy::unwrap_used,
    reason = "fixture assertions fail immediately on missing test scope"
)]
fn project_task_facts_preserve_accepted_results_and_reject_unbounded_or_inconsistent_snapshots() {
    use crate::v1::{ProjectStatusSnapshot, ProjectTaskFact};
    let id = identity();
    let binding = binding(&id.id);
    let fact = ProjectTaskFact {
        task_id: "task:00000000-0000-7000-8000-000000000001".into(),
        attempt_id: "attempt:00000000-0000-7000-8000-000000000002".into(),
        coding_state: "cancelled".into(),
        accepted_coding_state: "completed".into(),
        provider_state: "uncertain".into(),
        quality_state: "interrupted".into(),
        ..Default::default()
    };
    assert!(valid_project_task(&fact));
    let snapshot = ProjectStatusSnapshot {
        snapshot_id: id.id.clone(),
        project_id: binding.project_id.clone(),
        objective: "Facts".into(),
        status: "registered".into(),
        context_revision: binding.context_revision,
        project_resource_version: 1,
        tasks: vec![fact.clone()],
        total_tasks: Some(1),
        has_more_tasks: false,
    };
    let payload = ConversationTurnRequested {
        binding: Some(binding),
        project_status: Some(snapshot.clone()),
        ..Default::default()
    };
    assert!(validate_project_snapshot(&payload, payload.binding.as_ref().unwrap(), &id).is_ok());
    for change in 0..7 {
        let mut altered = snapshot.clone();
        match change {
            0 => altered.total_tasks = Some(0),
            1 => altered.has_more_tasks = true,
            2 => altered.tasks.push(fact.clone()),
            3 => {
                altered.tasks = vec![fact.clone(); 21];
                altered.total_tasks = Some(21);
            }
            4 => altered.tasks[0].quality_artifact_digest = "bad-digest".into(),
            5 => altered.tasks[0].accepted_coding_state = "merged".into(),
            _ => altered.total_tasks = None,
        }
        let mut payload = payload.clone();
        payload.project_status = Some(altered);
        assert!(
            validate_project_snapshot(&payload, payload.binding.as_ref().unwrap(), &id).is_err()
        );
    }
    let mut reporting = fact;
    reporting.coding_state = "reporting".into();
    reporting.quality_state = "passed".into();
    assert!(valid_project_task(&reporting));
    reporting.accepted_coding_state = "failed".into();
    assert!(!valid_project_task(&reporting));
    let mut legacy = payload;
    let legacy_snapshot = legacy.project_status.as_mut().unwrap();
    legacy_snapshot.tasks.clear();
    legacy_snapshot.total_tasks = None;
    assert!(validate_project_snapshot(&legacy, legacy.binding.as_ref().unwrap(), &id).is_ok());
}
