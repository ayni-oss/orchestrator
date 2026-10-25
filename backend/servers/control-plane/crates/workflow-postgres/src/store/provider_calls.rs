//! Paid-call admission and cancellation share the conversation transaction lock.
use super::{StoreError, WorkflowStore, support};
use ayni_orchestrator_events::{
    v1::{
        ConversationContextRecord, ConversationTurnBinding, ConversationTurnReport,
        ProviderCallEvidence,
    },
    workflow::decode_command,
};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use tokio_postgres::Transaction;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRequest {
    pub command_bytes: Vec<u8>,
    pub action: String,
    #[serde(default)]
    pub progress: Option<ayni_orchestrator_events::progress::ExecutionProgress>,
    #[serde(default)]
    pub after_ordinal: u64,
    #[serde(default)]
    pub call_id: String,
    #[serde(default)]
    pub request_digest: String,
    #[serde(default)]
    pub context_digest: String,
    #[serde(default)]
    pub reserved_input_tokens: u64,
    #[serde(default)]
    pub reserved_output_tokens: u64,
    #[serde(default)]
    pub reserved_total_tokens: u64,
}

#[derive(Default, Serialize)]
pub struct AuthorityResponse {
    pub allowed: bool,
    pub cancellation_requested: bool,
    pub state: String,
    pub records: Vec<ConversationContextRecord>,
    pub next_after_ordinal: u64,
    pub context_digest: String,
    pub record_count: u64,
}

pub struct ProviderCall {
    pub turn_id: String,
    pub evidence: ProviderCallEvidence,
    pub state: String,
    pub safe_reason: String,
    pub cancellation_requested: bool,
    pub uncertainty_acknowledged: bool,
}

pub struct CancellationReceipt {
    pub command_id: String,
    pub turn_id: String,
    pub state: String,
    pub cancellation_requested: bool,
}

impl WorkflowStore {
    /// Resolve a trusted supervisor request against the exact accepted command.
    ///
    /// # Errors
    /// Rejects stale, unclaimed, cancelled or changed requests without sending.
    #[allow(
        clippy::too_many_lines,
        reason = "one transaction serializes exact command validation, cancellation and paid-call admission"
    )]
    pub async fn session_authority(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityResponse, StoreError> {
        let (_, _, command) = decode_command(&request.command_bytes)
            .map_err(|_| StoreError::InvalidInput("command"))?;
        let binding = command
            .binding
            .as_ref()
            .ok_or(StoreError::InvalidInput("binding"))?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::conversation(
            &tx,
            &binding.workspace_id,
            &binding.project_id,
            &binding.conversation_id,
        )
        .await?;
        tx.query_one(
            "SELECT id FROM workflow_conversations WHERE id=$1 FOR UPDATE",
            &[&binding.conversation_id],
        )
        .await?;
        let row=tx.query_opt("SELECT t.execution_state,d.exact_command_bytes,EXISTS(SELECT 1 FROM workflow_report_receipts r WHERE r.turn_id=t.id AND r.report_sequence=1 AND r.decision='accepted'),EXISTS(SELECT 1 FROM workflow_cancellation_receipts c WHERE c.turn_id=t.id AND c.cancellation_requested),t.failure_reason FROM workflow_turns t JOIN workflow_dispatch_intents d ON d.turn_id=t.id WHERE t.workspace_id=$1 AND t.project_id=$2 AND t.conversation_id=$3 AND t.id=$4 FOR UPDATE OF t", &[&binding.workspace_id,&binding.project_id,&binding.conversation_id,&binding.turn_id]).await?.ok_or(StoreError::NotFound)?;
        let retained: Option<Vec<u8>> = row.get(1);
        if retained.as_deref() != Some(request.command_bytes.as_slice())
            || !super::sessions::current_generation(&tx, &binding.turn_id).await?
        {
            return Err(StoreError::InvalidInput("stale_command"));
        }
        let execution: String = row.get(0);
        let claimed: bool = row.get(2);
        let cancelled: bool = row.get(3);
        let mut response = AuthorityResponse {
            allowed: claimed
                && execution == "processing"
                && !cancelled
                && row.get::<_, String>(4) != "provider_uncertain",
            cancellation_requested: cancelled,
            state: if cancelled {
                "cancelled".into()
            } else {
                execution
            },
            ..Default::default()
        };
        if request.action == "context" {
            let preparing: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM workflow_activities WHERE conversation_id=$1 AND turn_id=$2 AND stage='preparation')", &[&binding.conversation_id,&binding.turn_id]).await?.get(0);
            if response.allowed && request.after_ordinal == 0 && !preparing {
                super::acceptance::append_activity(
                    &tx,
                    binding,
                    "preparation",
                    "processing",
                    "preparation",
                    "",
                )
                .await?;
            }
            tx.commit().await?;
            driver.abort();
            let page = self
                .get_turn_context_inputs(
                    &binding.workspace_id,
                    &binding.project_id,
                    &binding.conversation_id,
                    &binding.turn_id,
                    request.after_ordinal,
                    100,
                )
                .await?;
            response.records = page.records;
            response.next_after_ordinal = page.next_after_ordinal;
            response.context_digest = page.manifest.digest;
            response.record_count = page.manifest.record_count;
            return Ok(response);
        }
        if request.action == "progress" {
            if response.allowed {
                let session = command
                    .session
                    .as_ref()
                    .ok_or(StoreError::InvalidInput("session"))?;
                let progress = request
                    .progress
                    .as_ref()
                    .ok_or(StoreError::InvalidInput("progress"))?;
                if session.generation != binding.executor_generation {
                    return Err(StoreError::InvalidInput("generation"));
                }
                super::progress::retain(&tx, binding, &session.session_id, progress).await?;
            }
            tx.commit().await?;
            driver.abort();
            return Ok(response);
        }
        let existing = tx
            .query_opt(
                "SELECT state,evidence FROM workflow_provider_calls WHERE turn_id=$1",
                &[&binding.turn_id],
            )
            .await?;
        if request.action == "poll" {
            if let Some(row) = existing {
                response.state = row.get(0);
            }
        } else if request.action == "admit" {
            admit_call(&tx, request, &command, &mut response, existing).await?;
        } else {
            return Err(StoreError::InvalidInput("authority_action"));
        }
        tx.commit().await?;
        driver.abort();
        Ok(response)
    }

    /// List factual call evidence without credentials, prompts or raw errors.
    ///
    /// # Errors
    /// Returns scope or storage errors.
    pub async fn list_provider_calls(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
    ) -> Result<Vec<ProviderCall>, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        support::conversation(&tx, workspace, project, conversation).await?;
        let rows=tx.query("SELECT p.turn_id,p.evidence,p.state,p.safe_reason,EXISTS(SELECT 1 FROM workflow_cancellation_receipts c WHERE c.turn_id=p.turn_id AND c.cancellation_requested),EXISTS(SELECT 1 FROM workflow_provider_uncertainty_receipts r WHERE r.turn_id=p.turn_id) FROM workflow_provider_calls p JOIN workflow_turns t ON t.id=p.turn_id WHERE t.workspace_id=$1 AND t.project_id=$2 AND t.conversation_id=$3 ORDER BY t.sequence DESC LIMIT 100", &[&workspace,&project,&conversation]).await?;
        let result = rows
            .iter()
            .map(|row| {
                let bytes: Vec<u8> = row.get(1);
                Ok(ProviderCall {
                    turn_id: row.get(0),
                    evidence: ProviderCallEvidence::decode(bytes.as_slice())
                        .map_err(|_| StoreError::CorruptState)?,
                    state: row.get(2),
                    safe_reason: row.get(3),
                    cancellation_requested: row.get(4),
                    uncertainty_acknowledged: row.get(5),
                })
            })
            .collect();
        tx.commit().await?;
        driver.abort();
        result
    }

    /// Retain one scoped cancellation receipt; terminal acceptance uses this lock too.
    ///
    /// # Errors
    /// Rejects changed retries and wrong scopes.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit parent scope is part of command authority"
    )]
    pub async fn cancel_turn(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
        turn: &str,
        key: &str,
    ) -> Result<CancellationReceipt, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        support::validate_id(turn, "turn_id")?;
        support::validate_key(key)?;
        let digest = support::request_digest(
            "CancelTurn",
            &[
                workspace.as_bytes(),
                project.as_bytes(),
                conversation.as_bytes(),
                turn.as_bytes(),
            ],
        )
        .to_vec();
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        tx.query_opt("SELECT id FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&workspace,&project,&conversation]).await?.ok_or(StoreError::NotFound)?;
        if let Some(row)=tx.query_opt("SELECT request_digest,command_id,turn_id,state,cancellation_requested FROM workflow_cancellation_receipts WHERE conversation_id=$1 AND idempotency_key=$2", &[&conversation,&key]).await? {
            if row.get::<_,Vec<u8>>(0)!=digest{return Err(StoreError::IdempotencyConflict);}
            let result=CancellationReceipt {command_id:row.get(1),turn_id:row.get(2),state:row.get(3),cancellation_requested:row.get(4)};tx.commit().await?;driver.abort();return Ok(result);
        }
        let row=tx.query_opt("SELECT execution_state FROM workflow_turns WHERE workspace_id=$1 AND project_id=$2 AND conversation_id=$3 AND id=$4 FOR UPDATE", &[&workspace,&project,&conversation,&turn]).await?.ok_or(StoreError::NotFound)?;
        let execution: String = row.get(0);
        let terminal = matches!(execution.as_str(), "completed" | "failed");
        let state = if terminal {
            execution.as_str()
        } else if execution == "not_started" {
            "cancelled"
        } else {
            "cancellation_requested"
        };
        let result = CancellationReceipt {
            command_id: support::new_id(),
            turn_id: turn.into(),
            state: state.into(),
            cancellation_requested: !terminal,
        };
        tx.execute(
            "DELETE FROM workflow_execution_progress WHERE turn_id=$1",
            &[&turn],
        )
        .await?;
        tx.execute("INSERT INTO workflow_cancellation_receipts(conversation_id,idempotency_key,request_digest,command_id,turn_id,state,cancellation_requested) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&conversation,&key,&digest,&result.command_id,&turn,&state,&result.cancellation_requested]).await?;
        if !terminal {
            let binding = ConversationTurnBinding {
                workspace_id: workspace.into(),
                project_id: project.into(),
                conversation_id: conversation.into(),
                turn_id: turn.into(),
                ..Default::default()
            };
            if execution == "not_started" {
                tx.execute("UPDATE workflow_turns SET execution_state='failed',failure_reason='cancelled',updated_at=clock_timestamp() WHERE id=$1", &[&turn]).await?;
                super::sessions::finish_turn(&tx, turn).await?;
            }
            super::acceptance::append_activity(
                &tx,
                &binding,
                "execution",
                if execution == "not_started" {
                    "failed"
                } else {
                    "processing"
                },
                "",
                if execution == "not_started" {
                    "cancelled"
                } else {
                    "cancellation_requested"
                },
            )
            .await?;
            let active:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM workflow_turns WHERE conversation_id=$1 AND execution_state IN ('not_started','processing') AND scheduling_state='eligible')", &[&conversation]).await?.get(0);
            if !active {
                super::acceptance::promote_successor(&tx, conversation).await?;
            }
        }
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}

async fn admit_call(
    tx: &Transaction<'_>,
    request: &AuthorityRequest,
    command: &ayni_orchestrator_events::v1::ConversationTurnRequested,
    response: &mut AuthorityResponse,
    existing: Option<tokio_postgres::Row>,
) -> Result<(), StoreError> {
    let binding = command
        .binding
        .as_ref()
        .ok_or(StoreError::InvalidInput("binding"))?;
    let config = command
        .frozen_configuration
        .as_ref()
        .ok_or(StoreError::InvalidInput("configuration"))?;
    let manifest = command
        .context_manifest
        .as_ref()
        .ok_or(StoreError::InvalidInput("context_manifest"))?;
    if ayni_orchestrator_events::workflow::binding_provider(&binding.execution_binding_revision)
        .is_none()
        || ayni_orchestrator_events::workflow::binding_provider(&binding.execution_binding_revision)
            != Some(config.provider.as_str())
        || request.call_id != binding.command_event_id
        || !valid_digest(&request.request_digest)
        || request.context_digest != manifest.digest
        || request.reserved_input_tokens == 0
        || request.reserved_input_tokens > config.max_input_tokens
        || request.reserved_output_tokens == 0
        || request.reserved_output_tokens > config.max_output_tokens
        || request.reserved_total_tokens > config.max_total_tokens
        || request
            .reserved_input_tokens
            .checked_add(request.reserved_output_tokens)
            != Some(request.reserved_total_tokens)
    {
        return Err(StoreError::InvalidInput("provider_call_binding"));
    }
    if let Some(row) = existing {
        let bytes: Vec<u8> = row.get(1);
        let retained =
            ProviderCallEvidence::decode(bytes.as_slice()).map_err(|_| StoreError::CorruptState)?;
        if retained.call_id != request.call_id || retained.request_digest != request.request_digest
        {
            return Err(StoreError::IdempotencyConflict);
        }
        response.allowed = false;
        response.state = row.get(0);
        if response.state == "sending" {
            response.state = "uncertain".into();
            tx.execute("UPDATE workflow_provider_calls SET state='uncertain',safe_reason='provider_uncertain' WHERE turn_id=$1", &[&binding.turn_id]).await?;
        }
    } else if response.allowed {
        let evidence = ProviderCallEvidence {
            call_id: request.call_id.clone(),
            provider: config.provider.clone(),
            model: config.model.clone(),
            configuration_revision_id: config.revision_id.clone(),
            context_digest: manifest.digest.clone(),
            request_digest: request.request_digest.clone(),
            reserved_input_tokens: request.reserved_input_tokens,
            reserved_output_tokens: request.reserved_output_tokens,
            reserved_total_tokens: request.reserved_total_tokens,
            state: "sending".into(),
            ..Default::default()
        };
        tx.execute("INSERT INTO workflow_provider_calls(turn_id,call_id,state,evidence) VALUES($1,$2,'sending',$3)", &[&binding.turn_id,&request.call_id,&evidence.encode_to_vec()]).await?;
        response.state = "sending".into();
        super::acceptance::append_activity(
            tx,
            binding,
            "provider_request",
            "processing",
            "",
            "provider_sending",
        )
        .await?;
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub(super) async fn cancellation_requested(
    tx: &Transaction<'_>,
    turn: &str,
) -> Result<bool, StoreError> {
    Ok(tx.query_one("SELECT EXISTS(SELECT 1 FROM workflow_cancellation_receipts WHERE turn_id=$1 AND cancellation_requested)", &[&turn]).await?.get(0))
}

/// Verify the report against the admitted call before settling it exactly once.
pub(super) async fn retain_evidence(
    tx: &Transaction<'_>,
    binding: &ConversationTurnBinding,
    report: &ConversationTurnReport,
) -> Result<bool, StoreError> {
    let Some(evidence) = report.provider_call.as_ref() else {
        let admitted: bool = tx
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM workflow_provider_calls WHERE turn_id=$1)",
                &[&binding.turn_id],
            )
            .await?
            .get(0);
        if admitted {
            return Err(StoreError::InvalidInput("provider_evidence_missing"));
        }
        return Ok(false);
    };
    let existing = tx
        .query_opt(
            "SELECT evidence FROM workflow_provider_calls WHERE turn_id=$1",
            &[&binding.turn_id],
        )
        .await?;
    let was_admitted = existing.is_some();
    if let Some(row) = existing {
        let bytes: Vec<u8> = row.get(0);
        let admitted =
            ProviderCallEvidence::decode(bytes.as_slice()).map_err(|_| StoreError::CorruptState)?;
        if evidence.call_id != admitted.call_id
            || evidence.provider != admitted.provider
            || evidence.model != admitted.model
            || evidence.configuration_revision_id != admitted.configuration_revision_id
            || evidence.context_digest != admitted.context_digest
            || evidence.request_digest != admitted.request_digest
            || evidence.reserved_input_tokens != admitted.reserved_input_tokens
            || evidence.reserved_output_tokens != admitted.reserved_output_tokens
            || evidence.reserved_total_tokens != admitted.reserved_total_tokens
        {
            return Err(StoreError::InvalidInput("provider_evidence_binding"));
        }
    } else {
        // Preparation failures and pre-send cancellations have no paid-call reservation.
        if evidence.state != "failed" && evidence.state != "cancelled" {
            return Err(StoreError::InvalidInput("provider_call_not_admitted"));
        }
        tx.execute("INSERT INTO workflow_provider_calls(turn_id,call_id,state,evidence) VALUES($1,$2,$3,$4)", &[&binding.turn_id,&evidence.call_id,&evidence.state,&evidence.encode_to_vec()]).await?;
    }
    for tool in &evidence.tool_invocations {
        super::acceptance::append_activity(
            tx,
            binding,
            "tool_execution",
            "processing",
            "",
            &tool.name,
        )
        .await?;
    }
    let settled = (!was_admitted && matches!(evidence.state.as_str(), "failed" | "cancelled"))
        || evidence.total_tokens.is_some();
    tx.execute("UPDATE workflow_provider_calls SET state=$2,evidence=$3,safe_reason=$4,reservation_settled=$5 WHERE turn_id=$1", &[&binding.turn_id,&evidence.state,&evidence.encode_to_vec(),&report.failure_reason,&settled]).await?;
    Ok(evidence.state == "uncertain")
}

/// Immutable receipt for an operator's explicit uncertain-call retirement.
pub struct UncertaintyReceipt {
    pub command_id: String,
    pub turn_id: String,
    pub call_id: String,
}

impl WorkflowStore {
    /// Close an explicitly reviewed uncertain turn and fence its session without
    /// rewriting provider evidence, settling unknown usage, or retrying a call.
    ///
    /// # Errors
    /// Rejects wrong scope, changed receipts, sending calls, missing signed
    /// uncertainty decisions, or unresolved outbox handoffs.
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "exact scoped identity and immutable receipt are checked in one transaction"
    )]
    pub async fn acknowledge_provider_uncertainty(
        &self,
        workspace: &str,
        project: &str,
        conversation: &str,
        turn: &str,
        call: &str,
        expected_digest: &str,
        key: &str,
        acknowledge_possible_charge: bool,
    ) -> Result<UncertaintyReceipt, StoreError> {
        support::validate_scope(workspace, project, conversation)?;
        support::validate_id(turn, "turn_id")?;
        support::validate_key(key)?;
        if !acknowledge_possible_charge {
            return Err(StoreError::InvalidInput("acknowledge_possible_charge"));
        }
        let digest = support::request_digest(
            "AcknowledgeProviderUncertainty",
            &[
                workspace.as_bytes(),
                project.as_bytes(),
                conversation.as_bytes(),
                turn.as_bytes(),
                call.as_bytes(),
                expected_digest.as_bytes(),
                &[1],
            ],
        )
        .to_vec();
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        tx.query_opt("SELECT id FROM workflow_conversations WHERE workspace_id=$1 AND project_id=$2 AND id=$3 FOR UPDATE", &[&workspace,&project,&conversation]).await?.ok_or(StoreError::NotFound)?;
        if let Some(row) = tx.query_opt("SELECT request_digest,command_id,turn_id,call_id FROM workflow_provider_uncertainty_receipts WHERE conversation_id=$1 AND idempotency_key=$2", &[&conversation,&key]).await? {
            let retained: Vec<u8> = row.get(0);
            if retained != digest { return Err(StoreError::IdempotencyConflict); }
            let result = UncertaintyReceipt { command_id:row.get(1), turn_id:row.get(2), call_id:row.get(3) };
            tx.commit().await?;
            driver.abort();
            return Ok(result);
        }
        let row=tx.query_opt("SELECT p.state,p.evidence,t.execution_state,d.exact_command_bytes,EXISTS(SELECT 1 FROM workflow_report_receipts r WHERE r.turn_id=t.id AND r.report_sequence=2 AND r.decision='accepted'),EXISTS(SELECT 1 FROM workflow_outbox o WHERE o.conversation_id=t.conversation_id AND o.state <> 'transferred') FROM workflow_turns t JOIN workflow_provider_calls p ON p.turn_id=t.id JOIN workflow_dispatch_intents d ON d.turn_id=t.id WHERE t.workspace_id=$1 AND t.project_id=$2 AND t.conversation_id=$3 AND t.id=$4", &[&workspace,&project,&conversation,&turn]).await?.ok_or(StoreError::NotFound)?;
        let binding = uncertainty_binding(&row, call, expected_digest)?;
        if !super::sessions::current_generation(&tx, turn).await? {
            return Err(StoreError::InvalidInput("session_generation"));
        }
        let receipt = UncertaintyReceipt {
            command_id: support::new_id(),
            turn_id: turn.into(),
            call_id: call.into(),
        };
        tx.execute("INSERT INTO workflow_provider_uncertainty_receipts(conversation_id,idempotency_key,request_digest,command_id,turn_id,call_id) VALUES($1,$2,$3,$4,$5,$6)", &[&conversation,&key,&digest,&receipt.command_id,&turn,&call]).await?;
        tx.execute("UPDATE workflow_turns SET execution_state='failed',failure_reason='provider_uncertain',updated_at=clock_timestamp() WHERE id=$1", &[&turn]).await?;
        tx.execute("UPDATE workflow_sessions s SET state='replaced' FROM workflow_turn_sessions t WHERE t.turn_id=$1 AND s.id=t.session_id AND s.generation=t.generation", &[&turn]).await?;
        super::acceptance::append_activity(
            &tx,
            &binding,
            "provider_uncertain",
            "failed",
            "",
            "provider_uncertainty_acknowledged",
        )
        .await?;
        super::acceptance::promote_successor(&tx, conversation).await?;
        tx.commit().await?;
        driver.abort();
        Ok(receipt)
    }
}

fn uncertainty_binding(
    row: &tokio_postgres::Row,
    call: &str,
    expected_digest: &str,
) -> Result<ConversationTurnBinding, StoreError> {
    let evidence = ProviderCallEvidence::decode(row.get::<_, Vec<u8>>(1).as_slice())
        .map_err(|_| StoreError::CorruptState)?;
    if evidence.call_id != call || evidence.request_digest != expected_digest {
        return Err(StoreError::InvalidInput("provider_call_binding"));
    }
    if row.get::<_, String>(0) != "uncertain"
        || row.get::<_, String>(2) != "processing"
        || !row.get::<_, bool>(4)
        || row.get::<_, bool>(5)
    {
        return Err(StoreError::InvalidInput("provider_uncertainty_not_ready"));
    }
    let bytes: Vec<u8> = row
        .get::<_, Option<Vec<u8>>>(3)
        .ok_or(StoreError::CorruptState)?;
    let (_, _, command) = decode_command(&bytes).map_err(|_| StoreError::CorruptState)?;
    let binding = command.binding.ok_or(StoreError::CorruptState)?;
    Ok(binding)
}
