use super::{EXECUTION_BINDING, StoreError};
use crate::model::{
    Activity, CommandReceipt, Conversation, DispatchState, ExecutionState, HistoryKind,
    HistoryRecord, Message, MessageRole, PresentationState, Project, ReceiptKind, SchedulingState,
    TerminalTurnSummary, Timestamp, Turn, Workspace,
};
use ayni_orchestrator_events::{
    EventIdentity,
    v1::{ConversationContextRecord, ConversationTurnBinding, ConversationTurnRequested},
    workflow::{APPLICATION_SOURCE, COMMAND_SUBJECT, WorkflowEventMetadata, encode_command},
};
use sha2::{Digest as _, Sha256};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const OPERATOR_PRINCIPAL: &str = "local-operator";

pub(super) struct ProposedReceipt<'a> {
    pub(super) command_id: &'a str,
    pub(super) kind: ReceiptKind,
    pub(super) parent_scope: &'a str,
    pub(super) idempotency_key: &'a str,
    pub(super) request_digest: &'a [u8; 32],
    pub(super) workspace_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) conversation_id: &'a str,
    pub(super) message_id: &'a str,
    pub(super) turn_id: &'a str,
    pub(super) resource_id: &'a str,
    pub(super) conversation_revision: i64,
    pub(super) presentation_state: PresentationState,
}

pub(super) async fn insert_receipt(
    transaction: &Transaction<'_>,
    receipt: &ProposedReceipt<'_>,
) -> Result<bool, StoreError> {
    let inserted = transaction
        .query_opt(
            "INSERT INTO workflow_command_receipts \
             (command_id, principal, method, parent_scope, idempotency_key, request_digest, kind, \
              workspace_id, project_id, conversation_id, message_id, turn_id, resource_id, \
              conversation_revision, presentation_state) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) \
             ON CONFLICT (principal, method, parent_scope, idempotency_key) DO NOTHING \
             RETURNING command_id",
            &[
                &receipt.command_id,
                &OPERATOR_PRINCIPAL,
                &method(receipt.kind),
                &receipt.parent_scope,
                &receipt.idempotency_key,
                &receipt.request_digest.as_slice(),
                &receipt.kind.as_str(),
                &receipt.workspace_id,
                &receipt.project_id,
                &receipt.conversation_id,
                &receipt.message_id,
                &receipt.turn_id,
                &receipt.resource_id,
                &receipt.conversation_revision,
                &receipt.presentation_state.as_str(),
            ],
        )
        .await?;
    Ok(inserted.is_some())
}

pub(super) async fn replay_receipt(
    transaction: &Transaction<'_>,
    method: &str,
    parent_scope: &str,
    idempotency_key: &str,
    digest: &[u8; 32],
) -> Result<(CommandReceipt, String), StoreError> {
    let Some((receipt, retained_digest, resource_id)) =
        find_receipt(transaction, method, parent_scope, idempotency_key)
            .await?
            .map(|(nested, resource)| (nested.0, nested.1, resource))
    else {
        return Err(StoreError::CorruptState);
    };
    ensure_digest(&retained_digest, digest)?;
    Ok((receipt, resource_id))
}

pub(super) async fn find_receipt(
    transaction: &Transaction<'_>,
    method: &str,
    parent_scope: &str,
    idempotency_key: &str,
) -> Result<Option<((CommandReceipt, Vec<u8>), String)>, StoreError> {
    transaction
        .query_opt(
            "SELECT command_id, kind, workspace_id, project_id, conversation_id, message_id, \
                    turn_id, workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at), \
                    conversation_revision, resume_cursor, presentation_state, request_digest, \
                    resource_id \
             FROM workflow_command_receipts \
             WHERE principal = $1 AND method = $2 AND parent_scope = $3 AND idempotency_key = $4",
            &[
                &OPERATOR_PRINCIPAL,
                &method,
                &parent_scope,
                &idempotency_key,
            ],
        )
        .await?
        .as_ref()
        .map(|row| Ok(((receipt_from_row(row)?, row.get(12)), row.get(13))))
        .transpose()
}

pub(super) async fn receipt_and_resource(
    transaction: &Transaction<'_>,
    command_id: &str,
) -> Result<(CommandReceipt, String), StoreError> {
    let row = transaction
        .query_one(
            "SELECT command_id, kind, workspace_id, project_id, conversation_id, message_id, \
                    turn_id, workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at), \
                    conversation_revision, resume_cursor, presentation_state, resource_id \
             FROM workflow_command_receipts WHERE command_id = $1",
            &[&command_id],
        )
        .await?;
    Ok((receipt_from_row(&row)?, row.get(12)))
}

pub(super) fn receipt_from_row(row: &Row) -> Result<CommandReceipt, StoreError> {
    let revision: i64 = row.get(9);
    Ok(CommandReceipt {
        command_id: row.get(0),
        kind: ReceiptKind::parse(row.get::<_, &str>(1)).ok_or(StoreError::CorruptState)?,
        workspace_id: row.get(2),
        project_id: row.get(3),
        conversation_id: row.get(4),
        message_id: row.get(5),
        turn_id: row.get(6),
        accepted_at: timestamp(row.get(7), row.get(8))?,
        conversation_revision: unsigned(revision)?,
        resume_cursor: row.get(10),
        presentation_state: PresentationState::parse(row.get::<_, &str>(11))
            .ok_or(StoreError::CorruptState)?,
    })
}

pub(super) async fn set_receipt_cursor(
    transaction: &Transaction<'_>,
    command_id: &str,
    conversation_id: &str,
    value: i64,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "UPDATE workflow_command_receipts SET resume_cursor = $2 WHERE command_id = $1",
            &[&command_id, &cursor(conversation_id, value)],
        )
        .await?;
    Ok(())
}

pub(super) async fn require_workspace(
    transaction: &Transaction<'_>,
    workspace_id: &str,
) -> Result<(), StoreError> {
    if transaction
        .query_opt(
            "SELECT 1 FROM workflow_workspaces WHERE id = $1",
            &[&workspace_id],
        )
        .await?
        .is_some()
    {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

pub(super) async fn require_project(
    transaction: &Transaction<'_>,
    workspace_id: &str,
    project_id: &str,
) -> Result<(), StoreError> {
    if transaction
        .query_opt(
            "SELECT 1 FROM workflow_projects WHERE workspace_id = $1 AND id = $2",
            &[&workspace_id, &project_id],
        )
        .await?
        .is_some()
    {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

pub(super) async fn workspace(
    transaction: &Transaction<'_>,
    id: &str,
) -> Result<Workspace, StoreError> {
    let row = transaction
        .query_opt(
            "SELECT id, display_name, workflow_ts_seconds(created_at), workflow_ts_nanos(created_at), \
                    workflow_ts_seconds(updated_at), workflow_ts_nanos(updated_at), resource_version \
             FROM workflow_workspaces WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    workspace_from_row(&row)
}

pub(super) fn workspace_from_row(row: &Row) -> Result<Workspace, StoreError> {
    Ok(Workspace {
        id: row.get(0),
        display_name: row.get(1),
        created_at: timestamp(row.get(2), row.get(3))?,
        updated_at: timestamp(row.get(4), row.get(5))?,
        resource_version: unsigned(row.get(6))?,
    })
}

pub(super) async fn project(
    transaction: &Transaction<'_>,
    workspace_id: &str,
    id: &str,
) -> Result<Project, StoreError> {
    let row = transaction
        .query_opt(
            "SELECT id, workspace_id, display_name, goal, \
                    workflow_ts_seconds(created_at), workflow_ts_nanos(created_at), \
                    workflow_ts_seconds(updated_at), workflow_ts_nanos(updated_at), resource_version \
             FROM workflow_projects WHERE workspace_id = $1 AND id = $2",
            &[&workspace_id, &id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    project_from_row(&row)
}

pub(super) fn project_from_row(row: &Row) -> Result<Project, StoreError> {
    Ok(Project {
        id: row.get(0),
        workspace_id: row.get(1),
        display_name: row.get(2),
        goal: row.get(3),
        created_at: timestamp(row.get(4), row.get(5))?,
        updated_at: timestamp(row.get(6), row.get(7))?,
        resource_version: unsigned(row.get(8))?,
    })
}

pub(super) async fn conversation(
    transaction: &Transaction<'_>,
    workspace_id: &str,
    project_id: &str,
    id: &str,
) -> Result<Conversation, StoreError> {
    let row = transaction
        .query_opt(
            "SELECT id, workspace_id, project_id, title, revision, \
                    workflow_ts_seconds(created_at), workflow_ts_nanos(created_at) \
             FROM workflow_conversations \
             WHERE workspace_id = $1 AND project_id = $2 AND id = $3",
            &[&workspace_id, &project_id, &id],
        )
        .await?
        .ok_or(StoreError::NotFound)?;
    conversation_from_row(&row)
}

pub(super) fn conversation_from_row(row: &Row) -> Result<Conversation, StoreError> {
    Ok(Conversation {
        id: row.get(0),
        workspace_id: row.get(1),
        project_id: row.get(2),
        title: row.get(3),
        revision: unsigned(row.get(4))?,
        created_at: timestamp(row.get(5), row.get(6))?,
    })
}

pub(super) struct MessageAdmission<'a> {
    pub(super) workspace_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) conversation_id: &'a str,
    pub(super) content: &'a str,
    pub(super) revision: i64,
    pub(super) turn_sequence: i64,
    pub(super) history_sequence: i64,
    pub(super) command_id: &'a str,
    pub(super) message_id: &'a str,
    pub(super) turn_id: &'a str,
    pub(super) activity_id: &'a str,
    pub(super) update_id: &'a str,
    pub(super) assignment_id: &'a str,
    pub(super) eligible: bool,
}

async fn record_message_and_turn(
    transaction: &Transaction<'_>,
    message: &MessageAdmission<'_>,
) -> Result<i64, StoreError> {
    transaction
        .execute(
            "UPDATE workflow_conversations SET revision = $4, next_turn_sequence = $5, \
                 next_history_sequence = $6 \
             WHERE workspace_id = $1 AND project_id = $2 AND id = $3",
            &[
                &message.workspace_id,
                &message.project_id,
                &message.conversation_id,
                &message.revision,
                &(message.turn_sequence + 1),
                &(message.history_sequence + 1),
            ],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO workflow_messages \
             (id, workspace_id, project_id, conversation_id, turn_id, role, content, \
              sequence, accepted_at) \
             SELECT $1, $2, $3, $4, $5, 'user', $6, $7, accepted_at \
             FROM workflow_command_receipts WHERE command_id = $8",
            &[
                &message.message_id,
                &message.workspace_id,
                &message.project_id,
                &message.conversation_id,
                &message.turn_id,
                &message.content,
                &message.history_sequence,
                &message.command_id,
            ],
        )
        .await?;
    let scheduling = if message.eligible {
        "eligible"
    } else {
        "waiting_for_predecessor"
    };
    let dispatch = if message.eligible {
        "pending"
    } else {
        "not_eligible"
    };
    let context_revision = if message.eligible {
        message.revision
    } else {
        0
    };
    let generation = i64::from(message.eligible);
    transaction
        .execute(
            "INSERT INTO workflow_turns \
             (id, workspace_id, project_id, conversation_id, input_message_id, sequence, \
              execution_binding_revision, context_revision, assignment_id, \
              executor_generation, scheduling_state, dispatch_state, execution_state, \
              accepted_at, updated_at) \
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
                    'not_started', accepted_at, accepted_at \
             FROM workflow_command_receipts WHERE command_id = $13",
            &[
                &message.turn_id,
                &message.workspace_id,
                &message.project_id,
                &message.conversation_id,
                &message.message_id,
                &message.turn_sequence,
                &EXECUTION_BINDING,
                &context_revision,
                &message.assignment_id,
                &generation,
                &scheduling,
                &dispatch,
                &message.command_id,
            ],
        )
        .await?;
    Ok(context_revision)
}

pub(super) async fn record_message_admission(
    transaction: &Transaction<'_>,
    message: MessageAdmission<'_>,
) -> Result<CommandReceipt, StoreError> {
    let context_revision = record_message_and_turn(transaction, &message).await?;
    super::configuration::freeze_turn_inputs(transaction, message.workspace_id, message.turn_id)
        .await?;
    transaction.execute("UPDATE workflow_turns t SET execution_binding_revision=CASE c.provider WHEN 'openai' THEN 'openai-responses-v1' ELSE 'scripted-v1' END FROM workflow_turn_inputs i JOIN workflow_agent_configurations c ON c.workspace_id=i.workspace_id AND c.revision_id=i.configuration_revision_id WHERE t.id=$1 AND i.turn_id=t.id", &[&message.turn_id]).await?;
    let waiting_on = if message.eligible {
        "delivery"
    } else {
        "predecessor"
    };
    transaction
        .execute(
            "INSERT INTO workflow_activities \
             (id, workspace_id, project_id, conversation_id, conversation_revision, turn_id, \
              stage, presentation_state, waiting_on, accepted_at) \
             SELECT $1, $2, $3, $4, $5, $6, 'admission', 'accepted', $7, accepted_at \
             FROM workflow_command_receipts WHERE command_id = $8",
            &[
                &message.activity_id,
                &message.workspace_id,
                &message.project_id,
                &message.conversation_id,
                &message.revision,
                &message.turn_id,
                &waiting_on,
                &message.command_id,
            ],
        )
        .await?;
    let affected = vec![message.message_id.to_owned(), message.turn_id.to_owned()];
    let cursor_value: i64 = transaction
        .query_one(
            "INSERT INTO workflow_updates \
             (id, activity_id, workspace_id, project_id, conversation_id, \
              conversation_revision, kind, affected_resource_ids, presentation_state, \
              waiting_on, accepted_at) \
             SELECT $1, $2, $3, $4, $5, $6, 'message_admitted', $7, 'accepted', $8, \
                    accepted_at \
             FROM workflow_command_receipts WHERE command_id = $9 \
             RETURNING cursor",
            &[
                &message.update_id,
                &message.activity_id,
                &message.workspace_id,
                &message.project_id,
                &message.conversation_id,
                &message.revision,
                &affected,
                &waiting_on,
                &message.command_id,
            ],
        )
        .await?
        .get(0);
    super::observation::notify(transaction, message.conversation_id).await?;
    set_receipt_cursor(
        transaction,
        message.command_id,
        message.conversation_id,
        cursor_value,
    )
    .await?;
    persist_dispatch(transaction, &message, context_revision).await?;
    Ok(receipt_and_resource(transaction, message.command_id)
        .await?
        .0)
}

async fn persist_dispatch(
    transaction: &Transaction<'_>,
    message: &MessageAdmission<'_>,
    context_revision: i64,
) -> Result<(), StoreError> {
    if message.eligible {
        materialize_command(
            transaction,
            CommandMaterialization {
                workspace_id: message.workspace_id,
                project_id: message.project_id,
                conversation_id: message.conversation_id,
                turn_id: message.turn_id,
                input_message_id: message.message_id,
                input_content: message.content,
                assignment_id: message.assignment_id,
                executor_generation: 1,
                context_revision,
                receipt_command_id: message.command_id,
            },
        )
        .await
    } else {
        transaction
            .execute(
                "INSERT INTO workflow_dispatch_intents \
                 (turn_id, workspace_id, project_id, conversation_id, eligible) \
                 VALUES ($1, $2, $3, $4, false)",
                &[
                    &message.turn_id,
                    &message.workspace_id,
                    &message.project_id,
                    &message.conversation_id,
                ],
            )
            .await?;
        Ok(())
    }
}

pub(super) struct CommandMaterialization<'a> {
    pub(super) workspace_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) conversation_id: &'a str,
    pub(super) turn_id: &'a str,
    pub(super) input_message_id: &'a str,
    pub(super) input_content: &'a str,
    pub(super) assignment_id: &'a str,
    pub(super) executor_generation: u64,
    pub(super) context_revision: i64,
    pub(super) receipt_command_id: &'a str,
}

async fn command_context(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    turn_id: &str,
) -> Result<Vec<ConversationContextRecord>, StoreError> {
    let rows = transaction
        .query(
            "SELECT recent.id, recent.sequence, recent.role, recent.content \
             FROM ( \
               SELECT message.id, message.sequence, message.role, message.content, \
                      message_turn.sequence AS turn_sequence \
               FROM workflow_messages message \
               JOIN workflow_turns message_turn \
                 ON message_turn.conversation_id = message.conversation_id \
                AND message_turn.id = message.turn_id \
               JOIN workflow_turns target \
                 ON target.conversation_id = message.conversation_id AND target.id = $2 \
               WHERE message.conversation_id = $1 AND message_turn.sequence <= target.sequence \
               ORDER BY message_turn.sequence DESC, \
                 CASE message.role WHEN 'assistant' THEN 0 ELSE 1 END, message.sequence DESC \
               LIMIT 100 \
             ) recent \
             ORDER BY recent.turn_sequence, \
               CASE recent.role WHEN 'user' THEN 0 ELSE 1 END, recent.sequence",
            &[&conversation_id, &turn_id],
        )
        .await?;
    rows.iter()
        .map(|row| {
            Ok(ConversationContextRecord {
                message_id: row.get(0),
                sequence: unsigned(row.get(1))?,
                role: row.get(2),
                content: row.get(3),
            })
        })
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "command bytes, dispatch intent and outbox are materialized atomically"
)]
pub(super) async fn materialize_command(
    transaction: &Transaction<'_>,
    command: CommandMaterialization<'_>,
) -> Result<(), StoreError> {
    let session = super::sessions::bind_turn(transaction, command.turn_id).await?;
    let executor_generation = session
        .as_ref()
        .map_or(command.executor_generation, |scope| scope.generation);
    let event_id = new_id();
    let context = command_context(transaction, command.conversation_id, command.turn_id).await?;
    let frozen_configuration =
        super::configuration::execution_inputs(transaction, command.turn_id).await?;
    let context_manifest = super::context_manifest::freeze_if_configured(
        transaction,
        &command,
        frozen_configuration.is_some(),
    )
    .await?;
    let produced_at: String = transaction
        .query_one(
            "SELECT to_char(accepted_at AT TIME ZONE 'UTC', \
                    'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
             FROM workflow_command_receipts WHERE command_id = $1",
            &[&command.receipt_command_id],
        )
        .await?
        .get(0);
    let execution_binding_revision: String = transaction
        .query_one(
            "SELECT execution_binding_revision FROM workflow_turns WHERE id=$1",
            &[&command.turn_id],
        )
        .await?
        .get(0);
    let mut payload = ConversationTurnRequested {
        binding: Some(ConversationTurnBinding {
            workspace_id: command.workspace_id.to_owned(),
            project_id: command.project_id.to_owned(),
            conversation_id: command.conversation_id.to_owned(),
            turn_id: command.turn_id.to_owned(),
            assignment_id: command.assignment_id.to_owned(),
            executor_generation,
            execution_binding_revision,
            context_revision: unsigned(command.context_revision)?,
            command_event_id: event_id.clone(),
            command_digest: String::new(),
        }),
        input_message_id: command.input_message_id.to_owned(),
        input_content: command.input_content.to_owned(),
        context,
        frozen_configuration,
        context_manifest,
        session,
    };
    let identity = EventIdentity {
        source: APPLICATION_SOURCE.to_owned(),
        id: event_id.clone(),
    };
    let metadata = WorkflowEventMetadata {
        produced_at,
        traceparent: traceparent(),
    };
    let bytes =
        encode_materialized_command(&identity, &metadata, &mut payload, command.input_message_id)?;
    let dispatch_changed = transaction
        .execute(
            "INSERT INTO workflow_dispatch_intents \
             (turn_id, workspace_id, project_id, conversation_id, eligible, command_event_id, \
              exact_command_bytes) VALUES ($1, $2, $3, $4, true, $5, $6) \
             ON CONFLICT (turn_id) DO UPDATE SET eligible = true, \
              command_event_id = EXCLUDED.command_event_id, \
              exact_command_bytes = EXCLUDED.exact_command_bytes, updated_at = clock_timestamp() \
             WHERE NOT workflow_dispatch_intents.eligible \
               AND workflow_dispatch_intents.command_event_id = '' \
               AND workflow_dispatch_intents.exact_command_bytes IS NULL",
            &[
                &command.turn_id,
                &command.workspace_id,
                &command.project_id,
                &command.conversation_id,
                &event_id,
                &bytes,
            ],
        )
        .await?;
    if dispatch_changed != 1 {
        return Err(StoreError::CorruptState);
    }
    transaction
        .execute(
            "INSERT INTO workflow_outbox \
             (id, workspace_id, project_id, conversation_id, turn_id, subject, event_bytes) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &event_id,
                &command.workspace_id,
                &command.project_id,
                &command.conversation_id,
                &command.turn_id,
                &COMMAND_SUBJECT,
                &bytes,
            ],
        )
        .await?;
    Ok(())
}

pub(super) fn message_from_row(row: &Row) -> Result<Message, StoreError> {
    Ok(Message {
        id: row.get(0),
        conversation_id: row.get(1),
        turn_id: row.get(2),
        role: MessageRole::parse(row.get::<_, &str>(3)).ok_or(StoreError::CorruptState)?,
        content: row.get(4),
        sequence: unsigned(row.get(5))?,
        accepted_at: timestamp(row.get(6), row.get(7))?,
    })
}

pub(super) fn turn_from_row(row: &Row) -> Result<Turn, StoreError> {
    Ok(Turn {
        id: row.get(0),
        conversation_id: row.get(1),
        input_message_id: row.get(2),
        sequence: unsigned(row.get(3))?,
        execution_binding_revision: row.get(4),
        context_revision: unsigned(row.get(5))?,
        assignment_id: row.get(6),
        executor_generation: unsigned(row.get(7))?,
        scheduling_state: SchedulingState::parse(row.get::<_, &str>(8))
            .ok_or(StoreError::CorruptState)?,
        dispatch_state: DispatchState::parse(row.get::<_, &str>(9))
            .ok_or(StoreError::CorruptState)?,
        execution_state: ExecutionState::parse(row.get::<_, &str>(10))
            .ok_or(StoreError::CorruptState)?,
        response_message_id: row.get(11),
        failure_reason: row.get(12),
        accepted_at: timestamp(row.get(13), row.get(14))?,
        updated_at: timestamp(row.get(15), row.get(16))?,
    })
}

pub(super) fn terminal_from_row(row: &Row) -> Result<TerminalTurnSummary, StoreError> {
    let execution = ExecutionState::parse(row.get::<_, &str>(3)).ok_or(StoreError::CorruptState)?;
    let presentation_state = match execution {
        ExecutionState::Completed => PresentationState::Completed,
        ExecutionState::Failed => PresentationState::Failed,
        _ => return Err(StoreError::CorruptState),
    };
    Ok(TerminalTurnSummary {
        id: row.get(0),
        sequence: unsigned(row.get(1))?,
        input_message_id: row.get(2),
        presentation_state,
        response_message_id: row.get(4),
        safe_failure_reason: row.get(5),
        accepted_at: timestamp(row.get(6), row.get(7))?,
        terminal_at: timestamp(row.get(8), row.get(9))?,
    })
}

pub(super) fn activity_from_row(row: &Row) -> Result<Activity, StoreError> {
    let cursor_value: i64 = row.get(3);
    let conversation_id: String = row.get(1);
    Ok(Activity {
        id: row.get(0),
        conversation_id: conversation_id.clone(),
        conversation_revision: unsigned(row.get(2))?,
        cursor: cursor(&conversation_id, cursor_value),
        turn_id: row.get(4),
        stage: row.get(5),
        presentation_state: PresentationState::parse(row.get::<_, &str>(6))
            .ok_or(StoreError::CorruptState)?,
        waiting_on: row.get(7),
        safe_reason: row.get(8),
        accepted_at: timestamp(row.get(9), row.get(10))?,
        safe_detail: row.get(11),
    })
}

pub(super) fn history_from_message_row(row: &Row) -> Result<HistoryRecord, StoreError> {
    Ok(HistoryRecord {
        sequence: unsigned(row.get(0))?,
        id: row.get(1),
        kind: HistoryKind::Message,
        role_or_state: row.get(2),
        content_or_outcome: row.get(3),
        source_turn_id: row.get(4),
        accepted_at: timestamp(row.get(5), row.get(6))?,
    })
}

pub(super) const fn method(kind: ReceiptKind) -> &'static str {
    match kind {
        ReceiptKind::CreateWorkspace => "CreateWorkspace",
        ReceiptKind::CreateProject => "CreateProject",
        ReceiptKind::CreateConversation => "CreateConversation",
        ReceiptKind::SendMessage => "SendMessage",
    }
}

pub(super) fn validate_key(value: &str) -> Result<(), StoreError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(StoreError::InvalidInput("idempotency_key"))
    } else {
        Ok(())
    }
}

pub(super) fn validate_name(value: &str, field: &'static str) -> Result<(), StoreError> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > 256
        || value.chars().any(char::is_control)
    {
        Err(StoreError::InvalidInput(field))
    } else {
        Ok(())
    }
}

pub(super) fn validate_text(
    value: &str,
    maximum: usize,
    allow_empty: bool,
    field: &'static str,
) -> Result<(), StoreError> {
    if (!allow_empty && value.is_empty())
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        Err(StoreError::InvalidInput(field))
    } else {
        Ok(())
    }
}

pub(super) fn validate_id(value: &str, field: &'static str) -> Result<(), StoreError> {
    let parsed = Uuid::parse_str(value).map_err(|_| StoreError::InvalidInput(field))?;
    if parsed.get_version_num() == 7 && parsed.to_string() == value {
        Ok(())
    } else {
        Err(StoreError::InvalidInput(field))
    }
}

pub(super) fn validate_scope(
    workspace_id: &str,
    project_id: &str,
    conversation_id: &str,
) -> Result<(), StoreError> {
    validate_id(workspace_id, "workspace_id")?;
    validate_id(project_id, "project_id")?;
    if !conversation_id.is_empty() {
        validate_id(conversation_id, "conversation_id")?;
    }
    Ok(())
}

pub(super) fn request_digest(method: &str, values: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(
        u64::try_from(method.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    digest.update(method.as_bytes());
    for value in values {
        digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(value);
    }
    digest.finalize().into()
}

pub(super) fn ensure_digest(retained: &[u8], expected: &[u8; 32]) -> Result<(), StoreError> {
    if retained == expected {
        Ok(())
    } else {
        Err(StoreError::IdempotencyConflict)
    }
}

pub(super) fn new_id() -> String {
    Uuid::now_v7().to_string()
}

pub(super) fn cursor(conversation_id: &str, value: i64) -> String {
    format!("workflow-cursor-v1:{conversation_id}:{value}")
}

pub(super) fn traceparent() -> String {
    let trace = Uuid::now_v7().simple().to_string();
    let span = Uuid::now_v7().simple().to_string();
    format!("00-{trace}-{}-01", &span[..16])
}

pub(super) fn timestamp(seconds: i64, nanos: i64) -> Result<Timestamp, StoreError> {
    let nanos = i32::try_from(nanos).map_err(|_| StoreError::CorruptState)?;
    if !(0..1_000_000_000).contains(&nanos) {
        return Err(StoreError::CorruptState);
    }
    Ok(Timestamp { seconds, nanos })
}

pub(super) fn unsigned(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::CorruptState)
}

fn encode_materialized_command(
    identity: &EventIdentity,
    metadata: &WorkflowEventMetadata,
    payload: &mut ConversationTurnRequested,
    input_message_id: &str,
) -> Result<Vec<u8>, StoreError> {
    loop {
        if let Some(manifest) = payload.context_manifest.as_mut() {
            manifest.inline_complete = manifest.record_count
                == u64::try_from(payload.context.len()).map_err(|_| StoreError::CorruptState)?;
        }
        match encode_command(identity, metadata, payload) {
            Ok(bytes) => return Ok(bytes),
            Err(ayni_orchestrator_events::ContractError::Oversized) => {
                let Some(oldest) = payload
                    .context
                    .iter()
                    .position(|record| record.message_id != input_message_id)
                else {
                    return Err(StoreError::Capacity("command"));
                };
                payload.context.remove(oldest);
            }
            Err(_) => return Err(StoreError::CorruptState),
        }
    }
}
