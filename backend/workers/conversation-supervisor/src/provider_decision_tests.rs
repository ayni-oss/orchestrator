use super::*;

#[test]
fn provider_uncertainty_and_cancellation_decisions_complete_retained_handoff()
-> Result<(), LedgerError> {
    for uncertain in [true, false] {
        let directory = TempDir::new().map_err(|_| LedgerError::Invalid)?;
        let mut command = command_fixture(1, COMMAND_ID)?;
        command.binding.execution_binding_revision = "openai-responses-v1".into();
        let (_, metadata, mut payload) =
            ayni_orchestrator_events::workflow::decode_command(&command.bytes)
                .map_err(|_| LedgerError::Invalid)?;
        payload.binding = Some(command.binding.clone());
        command.bytes = encode_command(&command.identity, &metadata, &payload)
            .map_err(|_| LedgerError::Invalid)?;
        let claim = claim_fixture(&command, WINNER, &WINNER_SEED, CLAIM_ID)?;
        let mut ledger = open(&directory, LedgerLimits::m1_default(), WINNER, &WINNER_SEED)?;
        ledger.retain_command_and_claim(&command.command(), &claim.report(1))?;
        let accepted = decision_bytes(&command, &claim, WINNER, WINNER, true, None, None)?;
        ledger.record_decision(&Decision {
            exact_bytes: &accepted,
        })?;
        ledger.acknowledge_decision(&claim.identity.source, &claim.identity.id)?;
        let terminal = provider_terminal_fixture(&command, uncertain)?;
        ledger.retain_terminal_report(
            &command.identity.source,
            &command.identity.id,
            &terminal.report(2),
        )?;
        let original = decision_bytes(&command, &terminal, WINNER, WINNER, true, None, None)?;
        let (identity, metadata, mut decision) =
            ayni_orchestrator_events::workflow::decode_decision(&original)
                .map_err(|_| LedgerError::Invalid)?;
        decision.turn_state = if uncertain { "processing" } else { "failed" }.into();
        let exact =
            encode_decision(&identity, &metadata, &decision).map_err(|_| LedgerError::Invalid)?;
        ledger.record_decision(&Decision {
            exact_bytes: &exact,
        })?;
        ledger.acknowledge_decision(&terminal.identity.source, &terminal.identity.id)?;
        assert!(!ledger.can_execute(&command.identity.source, &command.identity.id)?);
        assert!(ledger.pending_reports()?.is_empty());
        assert!(ledger.can_retire()?);
    }
    let fixture = ConversationTurnReport {
        kind: ConversationTurnReportKind::Failed.into(),
        failure_reason: "script_failed".into(),
        ..Default::default()
    };
    assert!(!decision_state_matches(
        true,
        ConversationTurnReportKind::Failed,
        "processing",
        &fixture
    ));
    Ok(())
}

fn provider_terminal_fixture(
    command: &CommandFixture,
    uncertain: bool,
) -> Result<ReportFixture, LedgerError> {
    let identity = EventIdentity {
        source: format!("ayni://orchestrator/supervisors/{WINNER}"),
        id: TERMINAL_ID.into(),
    };
    let public_key = SigningKey::from_bytes(&WINNER_SEED)
        .verifying_key()
        .to_bytes();
    let mut binding = command.binding.clone();
    binding.command_digest = exact_digest(&command.bytes);
    let mut payload = ConversationTurnReport {
        binding: Some(binding),
        kind: if uncertain {
            ConversationTurnReportKind::Failed
        } else {
            ConversationTurnReportKind::Completed
        }
        .into(),
        report_sequence: 2,
        executor_instance_id: WINNER.into(),
        signing_key_digest: signing_key_digest(&public_key),
        failure_reason: if uncertain { "provider_uncertain" } else { "" }.into(),
        response_content: if uncertain { "" } else { "retained response" }.into(),
        provider_call: Some(ayni_orchestrator_events::v1::ProviderCallEvidence {
            call_id: COMMAND_ID.into(),
            provider: "openai".into(),
            model: "fixture-model".into(),
            configuration_revision_id: "018f0f7b-6a4d-7a11-8d77-717273747576".into(),
            context_digest: format!("sha256:{}", "a".repeat(64)),
            request_digest: format!("sha256:{}", "b".repeat(64)),
            state: if uncertain { "uncertain" } else { "completed" }.into(),
            response_id: if uncertain { "" } else { "resp_fixture" }.into(),
            reported_model: if uncertain { "" } else { "fixture-model" }.into(),
            reserved_input_tokens: 10,
            reserved_output_tokens: 20,
            reserved_total_tokens: 30,
            total_tokens: if uncertain { None } else { Some(20) },
            ..Default::default()
        }),
        ..Default::default()
    };
    sign_report(&identity, &mut payload, &WINNER_SEED).map_err(|_| LedgerError::Invalid)?;
    let bytes =
        encode_report(&identity, &event_metadata(), &payload).map_err(|_| LedgerError::Invalid)?;
    Ok(ReportFixture { identity, bytes })
}
