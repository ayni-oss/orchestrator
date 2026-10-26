mod rejections;
use super::{StoreError, WorkflowStore};
use crate::{
    connect,
    model::ReportDecisionReceipt,
    store::support::{CommandMaterialization, materialize_command, new_id, traceparent},
};
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{
        ConversationReportDecision, ConversationReportDecisionKind, ConversationTurnBinding,
        ConversationTurnReport, ConversationTurnReportKind,
    },
    workflow::{
        APPLICATION_SOURCE, WorkflowEventMetadata, decision_subject, decode_report,
        encode_decision, verify_report_signature,
    },
};
use sha2::{Digest as _, Sha256};
use tokio_postgres::Transaction;

struct ParsedReport {
    identity: EventIdentity,
    report: ConversationTurnReport,
    digest: String,
}

struct ClaimAuthority {
    execution_state: String,
    dispatch_state: String,
    binding_revision: String,
    context_revision: i64,
    assignment_id: String,
    executor_generation: i64,
    command_event_id: String,
    command_bytes: Option<Vec<u8>>,
    accepted_executor: Option<String>,
}

struct TerminalAuthority {
    execution_state: String,
    binding_revision: String,
    context_revision: i64,
    assignment_id: String,
    executor_generation: i64,
    command_event_id: String,
    command_bytes: Option<Vec<u8>>,
    claim_bytes: Option<Vec<u8>>,
}

struct Decision<'a> {
    accepted: bool,
    safe_reason: &'a str,
    turn_state: &'a str,
    accepted_executor: Option<String>,
}

impl WorkflowStore {
    /// Accept or reject one signed sequence-one executor claim and durably enqueue
    /// its targeted decision. A report-consumer `ACK` belongs after this commits.
    ///
    /// # Errors
    /// Returns malformed input, unavailable authority, or retained-state errors.
    pub async fn accept_claim(
        &self,
        exact_report_bytes: &[u8],
    ) -> Result<ReportDecisionReceipt, StoreError> {
        let parsed = parse_report(exact_report_bytes, ConversationTurnReportKind::Started)?;
        let binding = parsed
            .report
            .binding
            .as_ref()
            .ok_or(StoreError::CorruptState)?;
        let (mut client, driver) = connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        if let Some(receipt) =
            rejections::detached_rejection(&transaction, &parsed, exact_report_bytes).await?
        {
            transaction.commit().await?;
            driver.abort();
            return Ok(receipt);
        }
        if let Some(receipt) = prior_receipt(&transaction, &parsed).await? {
            transaction.commit().await?;
            driver.abort();
            return Ok(receipt);
        }
        let authority = claim_authority(&transaction, binding).await?;
        let mut decision = decide_claim(&parsed, binding, &authority)?;
        if !super::sessions::current_generation(&transaction, &binding.turn_id).await? {
            decision.accepted = false;
            decision.safe_reason = "stale_generation";
        }
        let receipt = insert_decision(
            &transaction,
            exact_report_bytes,
            &parsed,
            binding,
            &decision,
        )
        .await?;
        if decision.accepted {
            let turn_changed = transaction
                .execute(
                    "UPDATE workflow_turns SET dispatch_state = 'transferred', \
                     execution_state = 'processing', updated_at = clock_timestamp() \
                     WHERE id = $1",
                    &[&binding.turn_id],
                )
                .await?;
            let outbox_changed = transaction
                .execute(
                    "UPDATE workflow_outbox SET state = 'transferred', updated_at = clock_timestamp() \
                     WHERE id = $1 AND event_bytes = $2",
                    &[&binding.command_event_id, &authority.command_bytes],
                )
                .await?;
            if turn_changed != 1 || outbox_changed != 1 {
                return Err(StoreError::CorruptState);
            }
            let session = transaction
                .query_opt(
                    "SELECT generation FROM workflow_turn_sessions WHERE turn_id=$1",
                    &[&binding.turn_id],
                )
                .await?;
            let (stage, waiting) = session.as_ref().map_or(("execution", ""), |row| {
                (
                    if row.get::<_, i64>(0) > 1 {
                        "agent_wakeup"
                    } else {
                        "provisioning"
                    },
                    "provisioning",
                )
            });
            append_activity(&transaction, binding, stage, "processing", waiting, "").await?;
        }
        transaction.commit().await?;
        driver.abort();
        Ok(receipt)
    }

    /// Accept or reject one terminal report from the accepted claimant. A
    /// completed report creates its assistant message in the same transaction.
    ///
    /// # Errors
    /// Returns malformed input, unavailable authority, or retained-state errors.
    pub async fn accept_terminal_report(
        &self,
        exact_report_bytes: &[u8],
    ) -> Result<ReportDecisionReceipt, StoreError> {
        let parsed = parse_terminal_report(exact_report_bytes)?;
        let binding = parsed
            .report
            .binding
            .as_ref()
            .ok_or(StoreError::CorruptState)?;
        let (mut client, driver) = connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        if let Some(receipt) =
            rejections::detached_rejection(&transaction, &parsed, exact_report_bytes).await?
        {
            transaction.commit().await?;
            driver.abort();
            return Ok(receipt);
        }
        if let Some(receipt) = prior_receipt(&transaction, &parsed).await? {
            transaction.commit().await?;
            driver.abort();
            return Ok(receipt);
        }
        let authority = terminal_authority(&transaction, binding).await?;
        let kind = ConversationTurnReportKind::try_from(parsed.report.kind)
            .map_err(|_| StoreError::InvalidInput("executor_report"))?;
        let mut decision = decide_terminal(&parsed, binding, &authority, kind)?;
        if !super::sessions::current_generation(&transaction, &binding.turn_id).await? {
            decision.accepted = false;
            decision.safe_reason = "stale_generation";
        }
        if decision.accepted {
            if parsed.report.failure_reason == "provider_uncertain" {
                decision.turn_state = "processing";
            } else if super::provider_calls::cancellation_requested(&transaction, &binding.turn_id)
                .await?
            {
                decision.turn_state = "failed";
            }
        }
        let outcome = prepare_outcome(&transaction, binding, &parsed, kind, &mut decision).await?;
        let receipt = insert_decision(
            &transaction,
            exact_report_bytes,
            &parsed,
            binding,
            &decision,
        )
        .await?;
        if decision.accepted {
            if let Some(proposal) = outcome.proposal {
                super::plans::proposals::retain(&transaction, binding, &proposal).await?;
            }
            apply_accepted_outcome(&transaction, binding, &outcome.report, outcome.kind).await?;
        }
        transaction.commit().await?;
        driver.abort();
        Ok(receipt)
    }
}

struct PreparedOutcome {
    report: ConversationTurnReport,
    kind: ConversationTurnReportKind,
    proposal: Option<super::plans::proposals::PreparedProposal>,
}

async fn prepare_outcome(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    parsed: &ParsedReport,
    kind: ConversationTurnReportKind,
    decision: &mut Decision<'_>,
) -> Result<PreparedOutcome, StoreError> {
    let mut outcome = parsed.report.clone();
    let mut outcome_kind = kind;
    let proposal = if decision.accepted
        && !super::provider_calls::cancellation_requested(transaction, &binding.turn_id).await?
    {
        match super::plans::proposals::prepare(
            transaction,
            binding,
            &parsed.report,
            &parsed.identity,
            &parsed.digest,
        )
        .await?
        {
            Ok(proposal) => proposal,
            Err(reason) => {
                outcome.response_content.clear();
                outcome.failure_reason = reason.into();
                outcome_kind = ConversationTurnReportKind::Failed;
                decision.turn_state = "failed";
                None
            }
        }
    } else {
        None
    };
    Ok(PreparedOutcome {
        report: outcome,
        kind: outcome_kind,
        proposal,
    })
}

async fn apply_accepted_outcome(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    report: &ConversationTurnReport,
    kind: ConversationTurnReportKind,
) -> Result<(), StoreError> {
    let uncertain = super::provider_calls::retain_evidence(transaction, binding, report).await?;
    let cancelled =
        super::provider_calls::cancellation_requested(transaction, &binding.turn_id).await?;
    if uncertain {
        transaction
            .execute(
                "DELETE FROM workflow_execution_progress WHERE turn_id=$1",
                &[&binding.turn_id],
            )
            .await?;
        transaction.execute("UPDATE workflow_turns SET failure_reason='provider_uncertain',updated_at=clock_timestamp() WHERE id=$1", &[&binding.turn_id]).await?;
        append_activity(
            transaction,
            binding,
            "provider_uncertain",
            "processing",
            "",
            "provider_uncertain",
        )
        .await?;
    } else {
        let mut terminal = report.clone();
        let terminal_kind = if cancelled {
            terminal.response_content.clear();
            terminal.failure_reason = "cancelled".into();
            ConversationTurnReportKind::Failed
        } else {
            kind
        };
        apply_terminal(transaction, binding, &terminal, terminal_kind).await?;
        super::sessions::finish_turn(transaction, &binding.turn_id).await?;
        promote_successor(transaction, &binding.conversation_id).await?;
    }
    Ok(())
}

fn parse_report(
    exact_bytes: &[u8],
    expected: ConversationTurnReportKind,
) -> Result<ParsedReport, StoreError> {
    let (identity, _, report) =
        decode_report(exact_bytes).map_err(|_| StoreError::InvalidInput("executor_report"))?;
    let kind = ConversationTurnReportKind::try_from(report.kind)
        .map_err(|_| StoreError::InvalidInput("executor_report"))?;
    if kind != expected {
        return Err(StoreError::InvalidInput("executor_report_kind"));
    }
    Ok(ParsedReport {
        identity,
        report,
        digest: digest(exact_bytes),
    })
}

fn parse_terminal_report(exact_bytes: &[u8]) -> Result<ParsedReport, StoreError> {
    let (identity, _, report) =
        decode_report(exact_bytes).map_err(|_| StoreError::InvalidInput("executor_report"))?;
    let kind = ConversationTurnReportKind::try_from(report.kind)
        .map_err(|_| StoreError::InvalidInput("executor_report"))?;
    if !matches!(
        kind,
        ConversationTurnReportKind::Completed | ConversationTurnReportKind::Failed
    ) {
        return Err(StoreError::InvalidInput("executor_report_kind"));
    }
    Ok(ParsedReport {
        identity,
        report,
        digest: digest(exact_bytes),
    })
}

async fn prior_receipt(
    transaction: &Transaction<'_>,
    parsed: &ParsedReport,
) -> Result<Option<ReportDecisionReceipt>, StoreError> {
    let row = transaction
        .query_opt(
            "SELECT report_digest, decision_event_id, decision, safe_reason \
             FROM workflow_report_receipts WHERE report_source = $1 AND report_id = $2",
            &[&parsed.identity.source, &parsed.identity.id],
        )
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<_, String>(0) != parsed.digest {
        return Err(StoreError::CorruptState);
    }
    Ok(Some(ReportDecisionReceipt {
        report_source: parsed.identity.source.clone(),
        report_id: parsed.identity.id.clone(),
        decision_event_id: row.get(1),
        accepted: row.get::<_, String>(2) == "accepted",
        safe_reason: row.get(3),
    }))
}

async fn claim_authority(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
) -> Result<ClaimAuthority, StoreError> {
    let row = transaction
        .query_opt(
            "SELECT t.execution_state, t.dispatch_state, t.execution_binding_revision, \
                    t.context_revision, t.assignment_id, t.executor_generation, \
                    d.command_event_id, d.exact_command_bytes, \
                    (SELECT r.executor_instance_id FROM workflow_report_receipts r \
                     WHERE r.turn_id = t.id AND r.report_sequence = 1 \
                       AND r.decision = 'accepted' LIMIT 1) \
             FROM workflow_turns t JOIN workflow_dispatch_intents d ON d.turn_id = t.id \
             WHERE t.workspace_id = $1 AND t.project_id = $2 \
               AND t.conversation_id = $3 AND t.id = $4 FOR UPDATE OF t",
            &[
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                &binding.turn_id,
            ],
        )
        .await?
        .ok_or(StoreError::InvalidInput("executor_report_scope"))?;
    Ok(ClaimAuthority {
        execution_state: row.get(0),
        dispatch_state: row.get(1),
        binding_revision: row.get(2),
        context_revision: row.get(3),
        assignment_id: row.get(4),
        executor_generation: row.get(5),
        command_event_id: row.get(6),
        command_bytes: row.get(7),
        accepted_executor: row.get(8),
    })
}

fn decide_claim<'a>(
    parsed: &ParsedReport,
    binding: &ConversationTurnBinding,
    authority: &'a ClaimAuthority,
) -> Result<Decision<'a>, StoreError> {
    let generation = i64::try_from(binding.executor_generation)
        .map_err(|_| StoreError::InvalidInput("executor_generation"))?;
    let binding_matches = binding.execution_binding_revision == authority.binding_revision
        && i64::try_from(binding.context_revision).ok() == Some(authority.context_revision)
        && binding.command_event_id == authority.command_event_id
        && authority
            .command_bytes
            .as_deref()
            .is_some_and(|bytes| binding.command_digest == digest(bytes));
    let safe_reason = if binding.assignment_id != authority.assignment_id
        || generation != authority.executor_generation
    {
        "stale_generation"
    } else if !binding_matches {
        "wrong_binding"
    } else if authority.execution_state != "not_started"
        || authority.dispatch_state == "transferred"
        || authority.accepted_executor.is_some()
    {
        "claim_lost"
    } else if verify_report_signature(&parsed.identity, &parsed.report, None).is_err() {
        "invalid_signature"
    } else {
        ""
    };
    Ok(Decision {
        accepted: safe_reason.is_empty(),
        safe_reason,
        turn_state: if safe_reason.is_empty() {
            "processing"
        } else if authority.execution_state == "not_started" {
            "queued"
        } else {
            authority.execution_state.as_str()
        },
        accepted_executor: authority.accepted_executor.clone(),
    })
}

async fn terminal_authority(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
) -> Result<TerminalAuthority, StoreError> {
    let row = transaction
        .query_opt(
            "SELECT t.execution_state, t.execution_binding_revision, t.context_revision, \
                    t.assignment_id, t.executor_generation, d.command_event_id, \
                    d.exact_command_bytes, \
                    (SELECT r.exact_report_bytes FROM workflow_report_receipts r \
                     WHERE r.turn_id = t.id AND r.report_sequence = 1 \
                       AND r.decision = 'accepted' LIMIT 1) \
             FROM workflow_turns t JOIN workflow_dispatch_intents d ON d.turn_id = t.id \
             WHERE t.workspace_id = $1 AND t.project_id = $2 \
               AND t.conversation_id = $3 AND t.id = $4 FOR UPDATE OF t",
            &[
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                &binding.turn_id,
            ],
        )
        .await?
        .ok_or(StoreError::InvalidInput("executor_report_scope"))?;
    Ok(TerminalAuthority {
        execution_state: row.get(0),
        binding_revision: row.get(1),
        context_revision: row.get(2),
        assignment_id: row.get(3),
        executor_generation: row.get(4),
        command_event_id: row.get(5),
        command_bytes: row.get(6),
        claim_bytes: row.get(7),
    })
}

fn decide_terminal<'a>(
    parsed: &ParsedReport,
    binding: &ConversationTurnBinding,
    authority: &'a TerminalAuthority,
    kind: ConversationTurnReportKind,
) -> Result<Decision<'a>, StoreError> {
    let claim = authority
        .claim_bytes
        .as_deref()
        .and_then(|bytes| decode_report(bytes).ok());
    let accepted_executor = claim
        .as_ref()
        .map(|(_, _, report)| report.executor_instance_id.clone());
    let generation = i64::try_from(binding.executor_generation)
        .map_err(|_| StoreError::InvalidInput("executor_generation"))?;
    let generation_matches = binding.assignment_id == authority.assignment_id
        && generation == authority.executor_generation;
    let binding_matches = binding.execution_binding_revision == authority.binding_revision
        && i64::try_from(binding.context_revision).ok() == Some(authority.context_revision)
        && binding.command_event_id == authority.command_event_id
        && authority
            .command_bytes
            .as_deref()
            .is_some_and(|bytes| binding.command_digest == digest(bytes));
    let signature_valid = claim.as_ref().is_some_and(|(_, _, claim)| {
        claim.executor_instance_id == parsed.report.executor_instance_id
            && claim
                .signing_public_key
                .as_slice()
                .try_into()
                .ok()
                .is_some_and(|key: [u8; 32]| {
                    verify_report_signature(&parsed.identity, &parsed.report, Some(&key)).is_ok()
                })
    });
    let safe_reason = if !generation_matches {
        "stale_generation"
    } else if !binding_matches {
        "wrong_binding"
    } else if authority.execution_state != "processing" {
        "terminal_conflict"
    } else if !signature_valid {
        "invalid_signature"
    } else {
        ""
    };
    let accepted = safe_reason.is_empty();
    let turn_state = if accepted {
        match kind {
            ConversationTurnReportKind::Completed => "completed",
            ConversationTurnReportKind::Failed => "failed",
            _ => return Err(StoreError::InvalidInput("executor_report_kind")),
        }
    } else {
        authority.execution_state.as_str()
    };
    Ok(Decision {
        accepted,
        safe_reason,
        turn_state,
        accepted_executor,
    })
}

async fn decision_fact(
    transaction: &Transaction<'_>,
    parsed: &ParsedReport,
    binding: &ConversationTurnBinding,
    decision: &Decision<'_>,
) -> Result<(String, Vec<u8>), StoreError> {
    let decision_id = new_id();
    let accepted_executor_instance_id = if decision.accepted {
        parsed.report.executor_instance_id.clone()
    } else {
        decision
            .accepted_executor
            .as_deref()
            .unwrap_or_default()
            .to_owned()
    };
    let payload = ConversationReportDecision {
        report_source: parsed.identity.source.clone(),
        report_id: parsed.identity.id.clone(),
        report_digest: parsed.digest.clone(),
        target_executor_instance_id: parsed.report.executor_instance_id.clone(),
        accepted_executor_instance_id,
        assignment_id: binding.assignment_id.clone(),
        executor_generation: binding.executor_generation,
        decision: if decision.accepted {
            ConversationReportDecisionKind::Accepted.into()
        } else {
            ConversationReportDecisionKind::Rejected.into()
        },
        safe_reason: decision.safe_reason.to_owned(),
        turn_state: decision.turn_state.to_owned(),
        workspace_id: binding.workspace_id.clone(),
        project_id: binding.project_id.clone(),
        conversation_id: binding.conversation_id.clone(),
        turn_id: binding.turn_id.clone(),
    };
    let produced_at: String = transaction
        .query_one(
            "SELECT to_char(clock_timestamp() AT TIME ZONE 'UTC', \
             'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
            &[],
        )
        .await?
        .get(0);
    let bytes = encode_decision(
        &EventIdentity {
            source: APPLICATION_SOURCE.to_owned(),
            id: decision_id.clone(),
        },
        &WorkflowEventMetadata {
            produced_at,
            traceparent: traceparent(),
        },
        &payload,
    )
    .map_err(|_| StoreError::CorruptState)?;
    Ok((decision_id, bytes))
}

async fn insert_decision(
    transaction: &Transaction<'_>,
    exact_report_bytes: &[u8],
    parsed: &ParsedReport,
    binding: &ConversationTurnBinding,
    decision: &Decision<'_>,
) -> Result<ReportDecisionReceipt, StoreError> {
    let (decision_id, decision_bytes) =
        decision_fact(transaction, parsed, binding, decision).await?;
    super::capacity::reserve_decision(transaction, !decision.accepted).await?;
    transaction
        .execute(
            "INSERT INTO workflow_report_receipts \
             (report_source, report_id, report_digest, exact_report_bytes, workspace_id, \
              project_id, conversation_id, turn_id, assignment_id, executor_generation, \
              report_sequence, executor_instance_id, decision, safe_reason, \
              decision_event_id, decision_bytes) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
            &[
                &parsed.identity.source,
                &parsed.identity.id,
                &parsed.digest,
                &exact_report_bytes,
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                &binding.turn_id,
                &binding.assignment_id,
                &i64::try_from(binding.executor_generation)
                    .map_err(|_| StoreError::InvalidInput("executor_generation"))?,
                &i64::try_from(parsed.report.report_sequence)
                    .map_err(|_| StoreError::InvalidInput("report_sequence"))?,
                &parsed.report.executor_instance_id,
                &if decision.accepted {
                    "accepted"
                } else {
                    "rejected"
                },
                &decision.safe_reason,
                &decision_id,
                &decision_bytes,
            ],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO workflow_outbox \
             (id, workspace_id, project_id, conversation_id, turn_id, subject, event_bytes) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[
                &decision_id,
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                &binding.turn_id,
                &decision_subject(&parsed.report.executor_instance_id)
                    .map_err(|_| StoreError::CorruptState)?,
                &decision_bytes,
            ],
        )
        .await?;
    Ok(ReportDecisionReceipt {
        report_source: parsed.identity.source.clone(),
        report_id: parsed.identity.id.clone(),
        decision_event_id: decision_id,
        accepted: decision.accepted,
        safe_reason: decision.safe_reason.to_owned(),
    })
}

async fn apply_terminal(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    report: &ConversationTurnReport,
    kind: ConversationTurnReportKind,
) -> Result<(), StoreError> {
    let (presentation, failure, response_id) = match kind {
        ConversationTurnReportKind::Completed => {
            let response_id = new_id();
            let sequence: i64 = transaction
                .query_one(
                    "UPDATE workflow_conversations \
                     SET next_history_sequence = next_history_sequence + 1 \
                     WHERE workspace_id = $1 AND project_id = $2 AND id = $3 \
                     RETURNING next_history_sequence - 1",
                    &[
                        &binding.workspace_id,
                        &binding.project_id,
                        &binding.conversation_id,
                    ],
                )
                .await?
                .get(0);
            transaction
                .execute(
                    "INSERT INTO workflow_messages \
                     (id, workspace_id, project_id, conversation_id, turn_id, role, content, sequence) \
                     VALUES ($1,$2,$3,$4,$5,'assistant',$6,$7)",
                    &[
                        &response_id,
                        &binding.workspace_id,
                        &binding.project_id,
                        &binding.conversation_id,
                        &binding.turn_id,
                        &report.response_content,
                        &sequence,
                    ],
                )
                .await?;
            ("completed", "", response_id)
        }
        ConversationTurnReportKind::Failed => {
            ("failed", report.failure_reason.as_str(), String::new())
        }
        _ => return Err(StoreError::InvalidInput("executor_report_kind")),
    };
    let changed = transaction
        .execute(
            "UPDATE workflow_turns SET execution_state = $2, response_message_id = NULLIF($3,''), \
             failure_reason = $4, updated_at = clock_timestamp() WHERE id = $1",
            &[&binding.turn_id, &presentation, &response_id, &failure],
        )
        .await?;
    if changed != 1 {
        return Err(StoreError::CorruptState);
    }
    transaction
        .execute(
            "DELETE FROM workflow_execution_progress WHERE turn_id=$1",
            &[&binding.turn_id],
        )
        .await?;
    append_activity(transaction, binding, "execution", presentation, "", failure).await
}

pub(super) async fn append_activity(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    stage: &str,
    presentation: &str,
    waiting_on: &str,
    safe_reason: &str,
) -> Result<(), StoreError> {
    append_activity_detail(
        transaction,
        binding,
        stage,
        presentation,
        waiting_on,
        safe_reason,
        "",
    )
    .await
}

pub(super) async fn append_activity_detail(
    transaction: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    stage: &str,
    presentation: &str,
    waiting_on: &str,
    safe_reason: &str,
    safe_detail: &str,
) -> Result<(), StoreError> {
    let revision: i64 = transaction
        .query_one(
            "UPDATE workflow_conversations SET revision = revision + 1 \
             WHERE workspace_id = $1 AND project_id = $2 AND id = $3 RETURNING revision",
            &[
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
            ],
        )
        .await?
        .get(0);
    let activity_id = new_id();
    transaction
        .execute(
            "INSERT INTO workflow_activities \
             (id, workspace_id, project_id, conversation_id, conversation_revision, turn_id, \
              stage, presentation_state, waiting_on, safe_reason, safe_detail) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            &[
                &activity_id,
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                &revision,
                &binding.turn_id,
                &stage,
                &presentation,
                &waiting_on,
                &safe_reason,
                &safe_detail,
            ],
        )
        .await?;
    let affected = vec![binding.turn_id.clone()];
    transaction
        .execute(
            "INSERT INTO workflow_updates \
             (id, activity_id, workspace_id, project_id, conversation_id, \
              conversation_revision, kind, affected_resource_ids, presentation_state, \
              waiting_on, safe_reason) \
             VALUES ($1,$2,$3,$4,$5,$6,'turn_state_changed',$7,$8,$9,$10)",
            &[
                &new_id(),
                &activity_id,
                &binding.workspace_id,
                &binding.project_id,
                &binding.conversation_id,
                &revision,
                &affected,
                &presentation,
                &waiting_on,
                &safe_reason,
            ],
        )
        .await?;
    super::observation::notify(transaction, &binding.conversation_id).await?;
    Ok(())
}

pub(super) async fn promote_successor(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<(), StoreError> {
    let Some(row) = transaction
        .query_opt(
            "SELECT t.id,t.workspace_id,t.project_id,t.input_message_id,m.content,r.command_id, \
                    c.revision \
             FROM workflow_turns t JOIN workflow_messages m ON m.id=t.input_message_id \
             JOIN workflow_command_receipts r ON r.turn_id=t.id \
             JOIN workflow_conversations c ON c.id=t.conversation_id \
             WHERE t.conversation_id=$1 AND t.execution_state='not_started' \
               AND t.scheduling_state='waiting_for_predecessor' \
             ORDER BY t.sequence LIMIT 1 FOR UPDATE OF t",
            &[&conversation_id],
        )
        .await?
    else {
        return Ok(());
    };
    let turn_id: String = row.get(0);
    let workspace_id: String = row.get(1);
    let project_id: String = row.get(2);
    let input_message_id: String = row.get(3);
    let input_content: String = row.get(4);
    let receipt_command_id: String = row.get(5);
    let context_revision: i64 = row.get(6);
    let assignment_id = new_id();
    let changed = transaction
        .execute(
            "UPDATE workflow_turns SET scheduling_state='eligible',dispatch_state='pending', \
             context_revision=$2,assignment_id=$3,executor_generation=1, \
             updated_at=clock_timestamp() WHERE id=$1 \
               AND execution_state='not_started' \
               AND scheduling_state='waiting_for_predecessor' \
               AND dispatch_state='not_eligible'",
            &[&turn_id, &context_revision, &assignment_id],
        )
        .await?;
    if changed != 1 {
        return Err(StoreError::CorruptState);
    }
    materialize_command(
        transaction,
        CommandMaterialization {
            workspace_id: &workspace_id,
            project_id: &project_id,
            conversation_id,
            turn_id: &turn_id,
            input_message_id: &input_message_id,
            input_content: &input_content,
            assignment_id: &assignment_id,
            executor_generation: 1,
            context_revision,
            receipt_command_id: &receipt_command_id,
        },
    )
    .await
}

fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

pub(super) async fn detached_report_decision(
    transaction: &Transaction<'_>,
    bytes: &[u8],
) -> Result<Option<ReportDecisionReceipt>, StoreError> {
    let parsed = parse_terminal_report(bytes)?;
    rejections::detached_rejection(transaction, &parsed, bytes).await
}
