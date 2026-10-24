mod acceptance;
mod capacity;
mod configuration;
mod context_manifest;
mod delivery;
mod delivery_failure;
mod discovery;
mod execution;
mod plans;
mod project_continuity;
pub mod provider_calls;
pub mod publications;
pub mod task_feedback;
pub mod task_recovery;
pub mod task_runs;
pub use delivery::DecisionAcknowledgement;
mod ordered_reports;
mod quarantine;
pub use quarantine::{QuarantinedEvent, TransportHealth};
pub mod observation;
mod operational;
pub mod sessions;
mod support;

use self::support::{
    MessageAdmission, ProposedReceipt, activity_from_row, conversation, cursor, ensure_digest,
    find_receipt, history_from_message_row, insert_receipt, message_from_row, new_id, project,
    receipt_and_resource, record_message_admission, replay_receipt, request_digest,
    require_project, require_workspace, set_receipt_cursor, terminal_from_row, turn_from_row,
    unsigned, validate_id, validate_key, validate_name, validate_scope, validate_text, workspace,
};
use crate::model::{
    CommandReceipt, Conversation, ConversationSnapshot, Created, HistoryPage, OutboxRecord,
    PresentationState, Project, ReceiptKind, Workspace,
};
use std::sync::Arc;
use thiserror::Error;
use tokio_postgres::IsolationLevel;

const EXECUTION_BINDING: &str = "scripted-v1";
const MAX_NONTERMINAL_TURNS: i64 = 100;
const MAX_HISTORY_LIMIT: u32 = 100;
const DEFAULT_HISTORY_LIMIT: u32 = 50;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("invalid workflow input: {0}")]
    InvalidInput(&'static str),
    #[error("workflow resource was not found")]
    NotFound,
    #[error("IDEMPOTENCY_CONFLICT")]
    IdempotencyConflict,
    #[error("CONFIGURATION_REVISION_CONFLICT")]
    ConfigurationRevisionConflict,
    #[error("TASK_EXECUTION_CONFLICT")]
    TaskExecutionConflict,
    #[error("TASK_ALREADY_PREPARED")]
    TaskAlreadyPrepared,
    #[error("PLAN_VERSION_CONFLICT")]
    PlanVersionConflict,
    #[error("PLAN_ALREADY_APPROVED")]
    PlanAlreadyApproved,
    #[error("PLAN_DIGEST_MISMATCH")]
    PlanDigestMismatch,
    #[error("PLAN_UNRESOLVED_SCOPE")]
    PlanUnresolvedScope,
    #[error("workflow capacity is exhausted: {0}")]
    Capacity(&'static str),
    #[error("WATCH_CURSOR_EXPIRED")]
    WatchCursorExpired,
    #[error("HISTORY_SNAPSHOT_EXPIRED")]
    HistorySnapshotExpired,
    #[error("retained workflow state is invalid")]
    CorruptState,
    #[error("authoritative PostgreSQL state is unavailable")]
    Database(#[source] tokio_postgres::Error),
}

impl From<tokio_postgres::Error> for StoreError {
    fn from(source: tokio_postgres::Error) -> Self {
        Self::Database(source)
    }
}

#[derive(Clone, Debug)]
pub struct WorkflowStore {
    database_url: Arc<str>,
}

impl WorkflowStore {
    #[must_use]
    pub fn new(database_url: impl Into<Arc<str>>) -> Self {
        Self {
            database_url: database_url.into(),
        }
    }

    /// Create a workspace or replay the original receipt for the same command key.
    ///
    /// # Errors
    /// Returns validation, idempotency, database, or retained-state errors.
    pub async fn create_workspace(
        &self,
        idempotency_key: &str,
        display_name: &str,
    ) -> Result<Created<Workspace>, StoreError> {
        validate_key(idempotency_key)?;
        validate_name(display_name, "display_name")?;
        let digest = request_digest("CreateWorkspace", &[display_name.as_bytes()]);
        let workspace_id = new_id();
        let command_id = new_id();
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        let proposed = ProposedReceipt {
            command_id: &command_id,
            kind: ReceiptKind::CreateWorkspace,
            parent_scope: "root",
            idempotency_key,
            request_digest: &digest,
            workspace_id: &workspace_id,
            project_id: "",
            conversation_id: "",
            message_id: "",
            turn_id: "",
            resource_id: &workspace_id,
            conversation_revision: 0,
            presentation_state: PresentationState::Accepted,
        };
        let inserted = insert_receipt(&transaction, &proposed).await?;
        let (receipt, resource_id) = if inserted {
            transaction
                .execute(
                    "INSERT INTO workflow_workspaces \
                     (id, display_name, created_at, updated_at) \
                     SELECT $1, $2, accepted_at, accepted_at \
                     FROM workflow_command_receipts WHERE command_id = $3",
                    &[&workspace_id, &display_name, &command_id],
                )
                .await?;
            configuration::initialize(&transaction, &workspace_id).await?;
            receipt_and_resource(&transaction, &command_id).await?
        } else {
            replay_receipt(
                &transaction,
                "CreateWorkspace",
                "root",
                idempotency_key,
                &digest,
            )
            .await?
        };
        let workspace = workspace(&transaction, &resource_id).await?;
        transaction.commit().await?;
        driver.abort();
        Ok(Created {
            resource: workspace,
            receipt,
        })
    }

    /// Create a project in an existing workspace or replay the original receipt.
    ///
    /// # Errors
    /// Returns validation, scope, idempotency, database, or retained-state errors.
    pub async fn create_project(
        &self,
        idempotency_key: &str,
        workspace_id: &str,
        display_name: &str,
        goal: &str,
    ) -> Result<Created<Project>, StoreError> {
        validate_key(idempotency_key)?;
        validate_id(workspace_id, "workspace_id")?;
        validate_name(display_name, "display_name")?;
        validate_text(goal, 16_384, true, "goal")?;
        let digest = request_digest(
            "CreateProject",
            &[
                workspace_id.as_bytes(),
                display_name.as_bytes(),
                goal.as_bytes(),
            ],
        );
        let project_id = new_id();
        let command_id = new_id();
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        require_workspace(&transaction, workspace_id).await?;
        let proposed = ProposedReceipt {
            command_id: &command_id,
            kind: ReceiptKind::CreateProject,
            parent_scope: workspace_id,
            idempotency_key,
            request_digest: &digest,
            workspace_id,
            project_id: &project_id,
            conversation_id: "",
            message_id: "",
            turn_id: "",
            resource_id: &project_id,
            conversation_revision: 0,
            presentation_state: PresentationState::Accepted,
        };
        let inserted = insert_receipt(&transaction, &proposed).await?;
        let (receipt, resource_id) = if inserted {
            transaction
                .execute(
                    "INSERT INTO workflow_projects \
                     (id, workspace_id, display_name, goal, created_at, updated_at) \
                     SELECT $1, $2, $3, $4, accepted_at, accepted_at \
                     FROM workflow_command_receipts WHERE command_id = $5",
                    &[
                        &project_id,
                        &workspace_id,
                        &display_name,
                        &goal,
                        &command_id,
                    ],
                )
                .await?;
            receipt_and_resource(&transaction, &command_id).await?
        } else {
            replay_receipt(
                &transaction,
                "CreateProject",
                workspace_id,
                idempotency_key,
                &digest,
            )
            .await?
        };
        let project = project(&transaction, workspace_id, &resource_id).await?;
        transaction.commit().await?;
        driver.abort();
        Ok(Created {
            resource: project,
            receipt,
        })
    }

    /// Create a conversation and its first durable activity and update.
    ///
    /// # Errors
    /// Returns validation, scope, idempotency, database, or retained-state errors.
    #[allow(
        clippy::too_many_lines,
        reason = "the atomic conversation admission transaction is kept linear for auditability"
    )]
    pub async fn create_conversation(
        &self,
        idempotency_key: &str,
        workspace_id: &str,
        project_id: &str,
        title: &str,
    ) -> Result<Created<Conversation>, StoreError> {
        validate_key(idempotency_key)?;
        validate_scope(workspace_id, project_id, "")?;
        validate_name(title, "title")?;
        let digest = request_digest(
            "CreateConversation",
            &[
                workspace_id.as_bytes(),
                project_id.as_bytes(),
                title.as_bytes(),
            ],
        );
        let conversation_id = new_id();
        let command_id = new_id();
        let activity_id = new_id();
        let update_id = new_id();
        let parent = format!("{workspace_id}/{project_id}");
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        require_project(&transaction, workspace_id, project_id).await?;
        let proposed = ProposedReceipt {
            command_id: &command_id,
            kind: ReceiptKind::CreateConversation,
            parent_scope: &parent,
            idempotency_key,
            request_digest: &digest,
            workspace_id,
            project_id,
            conversation_id: &conversation_id,
            message_id: "",
            turn_id: "",
            resource_id: &conversation_id,
            conversation_revision: 1,
            presentation_state: PresentationState::Accepted,
        };
        let inserted = insert_receipt(&transaction, &proposed).await?;
        let (receipt, resource_id) = if inserted {
            transaction
                .execute(
                    "INSERT INTO workflow_conversations \
                     (id, workspace_id, project_id, title, created_at) \
                     SELECT $1, $2, $3, $4, accepted_at \
                     FROM workflow_command_receipts WHERE command_id = $5",
                    &[
                        &conversation_id,
                        &workspace_id,
                        &project_id,
                        &title,
                        &command_id,
                    ],
                )
                .await?;
            transaction
                .execute(
                    "INSERT INTO workflow_activities \
                     (id, workspace_id, project_id, conversation_id, conversation_revision, \
                      stage, presentation_state, accepted_at) \
                     SELECT $1, $2, $3, $4, 1, 'admission', 'accepted', accepted_at \
                     FROM workflow_command_receipts WHERE command_id = $5",
                    &[
                        &activity_id,
                        &workspace_id,
                        &project_id,
                        &conversation_id,
                        &command_id,
                    ],
                )
                .await?;
            let cursor: i64 = transaction
                .query_one(
                    "INSERT INTO workflow_updates \
                     (id, activity_id, workspace_id, project_id, conversation_id, \
                      conversation_revision, kind, affected_resource_ids, presentation_state, \
                      accepted_at) \
                     SELECT $1, $2, $3, $4, $5, 1, 'conversation_created', ARRAY[$5], \
                            'accepted', accepted_at \
                     FROM workflow_command_receipts WHERE command_id = $6 \
                     RETURNING cursor",
                    &[
                        &update_id,
                        &activity_id,
                        &workspace_id,
                        &project_id,
                        &conversation_id,
                        &command_id,
                    ],
                )
                .await?
                .get(0);
            observation::notify(&transaction, &conversation_id).await?;
            set_receipt_cursor(&transaction, &command_id, &conversation_id, cursor).await?;
            receipt_and_resource(&transaction, &command_id).await?
        } else {
            replay_receipt(
                &transaction,
                "CreateConversation",
                &parent,
                idempotency_key,
                &digest,
            )
            .await?
        };
        let conversation =
            conversation(&transaction, workspace_id, project_id, &resource_id).await?;
        transaction.commit().await?;
        driver.abort();
        Ok(Created {
            resource: conversation,
            receipt,
        })
    }

    /// Admit one user message, turn, activity, update, and dispatch intent atomically.
    ///
    /// # Errors
    /// Returns validation, scope, capacity, idempotency, database, or retained-state errors.
    #[allow(
        clippy::too_many_lines,
        reason = "the atomic message admission transaction is kept linear for auditability"
    )]
    pub async fn send_message(
        &self,
        idempotency_key: &str,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
        content: &str,
    ) -> Result<CommandReceipt, StoreError> {
        validate_key(idempotency_key)?;
        validate_scope(workspace_id, project_id, conversation_id)?;
        validate_text(content, 16_384, false, "content")?;
        let parent = format!("{workspace_id}/{project_id}/{conversation_id}");
        let digest = request_digest(
            "SendMessage",
            &[
                workspace_id.as_bytes(),
                project_id.as_bytes(),
                conversation_id.as_bytes(),
                content.as_bytes(),
            ],
        );
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        let conversation_row = transaction
            .query_opt(
                "SELECT revision, next_turn_sequence, next_history_sequence \
                 FROM workflow_conversations \
                 WHERE workspace_id = $1 AND project_id = $2 AND id = $3 FOR UPDATE",
                &[&workspace_id, &project_id, &conversation_id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        if let Some((receipt, _)) =
            find_receipt(&transaction, "SendMessage", &parent, idempotency_key).await?
        {
            ensure_digest(&receipt.1, &digest)?;
            transaction.commit().await?;
            driver.abort();
            return Ok(receipt.0);
        }
        let nonterminal: i64 = transaction
            .query_one(
                "SELECT count(*) FROM workflow_turns \
                 WHERE conversation_id = $1 AND execution_state NOT IN ('completed', 'failed')",
                &[&conversation_id],
            )
            .await?
            .get(0);
        if nonterminal >= MAX_NONTERMINAL_TURNS {
            return Err(StoreError::Capacity("turn_queue"));
        }
        context_manifest::reserve_admission(&transaction, conversation_id, nonterminal).await?;
        capacity::reserve_admission(&transaction).await?;
        let current_revision: i64 = conversation_row.get(0);
        let turn_sequence: i64 = conversation_row.get(1);
        let history_sequence: i64 = conversation_row.get(2);
        let revision = current_revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        let eligible = nonterminal == 0;
        let command_id = new_id();
        let message_id = new_id();
        let turn_id = new_id();
        let activity_id = new_id();
        let update_id = new_id();
        let assignment_id = if eligible { new_id() } else { String::new() };
        let proposed = ProposedReceipt {
            command_id: &command_id,
            kind: ReceiptKind::SendMessage,
            parent_scope: &parent,
            idempotency_key,
            request_digest: &digest,
            workspace_id,
            project_id,
            conversation_id,
            message_id: &message_id,
            turn_id: &turn_id,
            resource_id: "",
            conversation_revision: revision,
            presentation_state: PresentationState::Accepted,
        };
        if !insert_receipt(&transaction, &proposed).await? {
            return Err(StoreError::CorruptState);
        }
        let receipt = record_message_admission(
            &transaction,
            MessageAdmission {
                workspace_id,
                project_id,
                conversation_id,
                content,
                revision,
                turn_sequence,
                history_sequence,
                command_id: &command_id,
                message_id: &message_id,
                turn_id: &turn_id,
                activity_id: &activity_id,
                update_id: &update_id,
                assignment_id: &assignment_id,
                eligible,
            },
        )
        .await?;
        transaction.commit().await?;
        driver.abort();
        Ok(receipt)
    }

    /// Mark one exact outbox record as durably stored by `JetStream`.
    ///
    /// This never transfers application authority: an accepted executor claim is
    /// still required before the command may leave `PostgreSQL` recovery ownership.
    ///
    /// # Errors
    /// Returns validation, database, or retained-state errors.
    pub async fn mark_outbox_stored(&self, id: &str, event_bytes: &[u8]) -> Result<(), StoreError> {
        self.mark_outbox_publication(id, event_bytes, 0).await
    }

    /// Retain the exact broker storage sequence for targeted ACK reconciliation.
    ///
    /// # Errors
    /// Returns database, input or retained-state errors.
    pub async fn mark_outbox_publication(
        &self,
        id: &str,
        event_bytes: &[u8],
        stream_sequence: u64,
    ) -> Result<(), StoreError> {
        self.mark_outbox_publication_at(
            id,
            event_bytes,
            stream_sequence,
            std::time::SystemTime::now(),
        )
        .await
    }

    /// Record a stored copy using the publication start time, preserving its age
    /// across duplicate broker acknowledgements for the same sequence.
    ///
    /// # Errors
    /// Returns invalid input, unavailable authority, or retained-state errors.
    pub async fn mark_outbox_publication_at(
        &self,
        id: &str,
        event_bytes: &[u8],
        stream_sequence: u64,
        published_at: std::time::SystemTime,
    ) -> Result<(), StoreError> {
        let published_at = published_at
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| StoreError::InvalidInput("publication_time"))?
            .as_secs_f64();
        let sequence = i64::try_from(stream_sequence)
            .map_err(|_| StoreError::InvalidInput("stream_sequence"))?;
        validate_id(id, "outbox_id")?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client.transaction().await?;
        // Observation updates follow the same conversation, Turn, outbox lock
        // order as admission and acceptance, including recovery activity.
        transaction
            .query_opt(
                "SELECT c.id FROM workflow_conversations c JOIN workflow_outbox o \
            ON o.conversation_id=c.id WHERE o.id=$1 FOR UPDATE OF c",
                &[&id],
            )
            .await?;
        let turn_before = transaction
            .query_opt(
                "SELECT t.dispatch_state FROM workflow_turns t JOIN workflow_outbox o \
            ON o.turn_id=t.id WHERE o.id=$1 FOR UPDATE OF t",
                &[&id],
            )
            .await?;
        let changed = transaction
            .execute(
                "UPDATE workflow_outbox SET state = 'stored', attempt_count = attempt_count + 1, \
                 updated_at = CASE WHEN state='pending' OR stream_sequence < $3 THEN to_timestamp($4::double precision) ELSE updated_at END, stream_sequence = GREATEST(stream_sequence,$3), last_safe_reason = '', next_attempt_at = clock_timestamp() + interval '30 seconds' \
                 WHERE id = $1 AND event_bytes = $2 AND state IN ('pending','stored')",
                &[&id, &event_bytes, &sequence, &published_at],
            )
            .await?;
        if changed > 1 {
            return Err(StoreError::CorruptState);
        }
        if changed == 1 {
            let turn_changed = transaction
                .execute(
                    "UPDATE workflow_turns SET dispatch_state = 'stored', \
                     updated_at = clock_timestamp() \
                     WHERE id = (SELECT turn_id FROM workflow_dispatch_intents WHERE command_event_id = $1) \
                       AND dispatch_state IN ('pending','degraded')",
                    &[&id],
                )
                .await?;
            if turn_changed > 1 {
                return Err(StoreError::CorruptState);
            }
            if turn_changed == 1
                && turn_before.is_some_and(|row| row.get::<_, String>(0) == "degraded")
            {
                delivery_failure::record_dispatch_recovery(&transaction, event_bytes).await?;
            }
        } else if delivery::mark_rejection_publication(
            &transaction,
            id,
            event_bytes,
            sequence,
            published_at,
        )
        .await?
        {
            // Detached rejections have no domain Turn to advance.
        } else {
            let retained: Option<bool> = transaction
                .query_opt(
                    "SELECT event_bytes = $2 AND state IN ('stored', 'transferred') \
                     FROM workflow_outbox WHERE id = $1",
                    &[&id, &event_bytes],
                )
                .await?
                .map(|row| row.get(0));
            if retained != Some(true) {
                return Err(StoreError::CorruptState);
            }
        }
        transaction.commit().await?;
        driver.abort();
        Ok(())
    }

    /// Read one repeatable-read conversation snapshot and establish a history boundary.
    ///
    /// # Errors
    /// Returns validation, scope, database, or retained-state errors.
    pub async fn get_snapshot(
        &self,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationSnapshot, StoreError> {
        validate_scope(workspace_id, project_id, conversation_id)?;
        for attempt in 0..8 {
            let result = self
                .get_snapshot_once(workspace_id, project_id, conversation_id)
                .await;
            let retry = matches!(&result,Err(StoreError::Database(error)) if error.code()==Some(&tokio_postgres::error::SqlState::T_R_SERIALIZATION_FAILURE));
            if !retry || attempt == 7 {
                return result;
            }
            // The retained history boundary's foreign key can race a conversation
            // update after the repeatable-read snapshot. Retry the whole rolled-back
            // projection, never admission, reports, or an external provider request.
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        unreachable!("bounded snapshot retry always returns its last result")
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the repeatable-read projection is kept in one transaction for auditability"
    )]
    async fn get_snapshot_once(
        &self,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationSnapshot, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await?;
        let conversation =
            conversation(&transaction, workspace_id, project_id, conversation_id).await?;
        let mut messages = transaction
            .query(
                "SELECT id, conversation_id, COALESCE(turn_id, ''), role, content, sequence, \
                        workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at) \
                 FROM workflow_messages WHERE conversation_id = $1 \
                 ORDER BY sequence DESC LIMIT 16",
                &[&conversation_id],
            )
            .await?
            .iter()
            .map(message_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        messages.reverse();
        let turns = transaction
            .query(
                "SELECT id, conversation_id, input_message_id, sequence, \
                        execution_binding_revision, context_revision, assignment_id, \
                        executor_generation, scheduling_state, dispatch_state, execution_state, \
                        COALESCE(response_message_id, ''), failure_reason, \
                        workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at), \
                        workflow_ts_seconds(updated_at), workflow_ts_nanos(updated_at) \
                 FROM workflow_turns WHERE conversation_id = $1 \
                   AND execution_state NOT IN ('completed', 'failed') ORDER BY sequence",
                &[&conversation_id],
            )
            .await?
            .iter()
            .map(turn_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let terminal = transaction
            .query(
                "SELECT id, sequence, input_message_id, execution_state, \
                        COALESCE(response_message_id, ''), failure_reason, \
                        workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at), \
                        workflow_ts_seconds(updated_at), workflow_ts_nanos(updated_at) \
                 FROM workflow_turns WHERE conversation_id = $1 \
                   AND execution_state IN ('completed', 'failed') \
                 ORDER BY sequence DESC LIMIT 16",
                &[&conversation_id],
            )
            .await?
            .iter()
            .map(terminal_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let counts = transaction
            .query_one(
                "SELECT (SELECT count(*) FROM workflow_messages WHERE conversation_id = $1), \
                        (SELECT count(*) FROM workflow_turns WHERE conversation_id = $1), \
                        COALESCE((SELECT max(sequence) FROM workflow_messages \
                                  WHERE conversation_id = $1), 0)",
                &[&conversation_id],
            )
            .await?;
        let message_count: i64 = counts.get(0);
        let turn_count: i64 = counts.get(1);
        let boundary: i64 = counts.get(2);
        transaction
            .execute(
                "DELETE FROM workflow_history_snapshots WHERE id IN ( \
                     SELECT id FROM workflow_history_snapshots \
                     WHERE expires_at <= clock_timestamp() \
                     ORDER BY expires_at LIMIT 1000 \
                 )",
                &[],
            )
            .await?;
        let snapshot_id = new_id();
        transaction
            .execute(
                "INSERT INTO workflow_history_snapshots \
                 (id, workspace_id, project_id, conversation_id, boundary_sequence, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, clock_timestamp() + interval '15 minutes')",
                &[
                    &snapshot_id,
                    &workspace_id,
                    &project_id,
                    &conversation_id,
                    &boundary,
                ],
            )
            .await?;
        let activity = transaction
            .query_opt(
                "SELECT a.id, a.conversation_id, a.conversation_revision, \
                        COALESCE(u.cursor, w.pruned_through_cursor, 0), a.turn_id, \
                        a.stage, a.presentation_state, a.waiting_on, a.safe_reason, \
                        workflow_ts_seconds(a.accepted_at), workflow_ts_nanos(a.accepted_at), a.safe_detail \
                 FROM workflow_activities a \
                 LEFT JOIN workflow_updates u ON u.activity_id = a.id \
                 LEFT JOIN workflow_watch_watermarks w ON w.conversation_id = a.conversation_id \
                 WHERE a.conversation_id = $1 ORDER BY a.conversation_revision DESC LIMIT 1",
                &[&conversation_id],
            )
            .await?
            .as_ref()
            .map(activity_from_row)
            .transpose()?;
        let cursor_value: i64 = transaction
            .query_one(
                "SELECT GREATEST( \
                    COALESCE((SELECT max(cursor) FROM workflow_updates WHERE conversation_id = $1), 0), \
                    COALESCE((SELECT pruned_through_cursor FROM workflow_watch_watermarks \
                              WHERE conversation_id = $1), 0))",
                &[&conversation_id],
            )
            .await?
            .get(0);
        let plans = plans::summaries(&transaction, conversation_id).await?;
        transaction.commit().await?;
        driver.abort();
        Ok(ConversationSnapshot {
            plans,
            workspace_id: workspace_id.to_owned(),
            project_id: project_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            conversation_revision: conversation.revision,
            recent_messages: messages,
            nonterminal_turns: turns,
            recent_terminal_turns: terminal,
            message_count: unsigned(message_count)?,
            turn_count: unsigned(turn_count)?,
            has_older_history: message_count > 16,
            history_snapshot_id: snapshot_id,
            current_activity: activity,
            resume_cursor: cursor(conversation_id, cursor_value),
            title: conversation.title,
            created_at: conversation.created_at,
        })
    }

    /// List one bounded page at a retained conversation-history boundary.
    ///
    /// # Errors
    /// Returns validation, scope, expiry, database, or retained-state errors.
    pub async fn list_history(
        &self,
        workspace_id: &str,
        project_id: &str,
        conversation_id: &str,
        snapshot_id: &str,
        before_sequence: u64,
        limit: u32,
    ) -> Result<HistoryPage, StoreError> {
        validate_scope(workspace_id, project_id, conversation_id)?;
        validate_id(snapshot_id, "snapshot_id")?;
        let limit = if limit == 0 {
            DEFAULT_HISTORY_LIMIT
        } else if limit <= MAX_HISTORY_LIMIT {
            limit
        } else {
            return Err(StoreError::InvalidInput("limit"));
        };
        let before = i64::try_from(before_sequence)
            .map_err(|_| StoreError::InvalidInput("before_sequence"))?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await?;
        let snapshot = transaction
            .query_opt(
                "SELECT boundary_sequence, expires_at > clock_timestamp() \
                 FROM workflow_history_snapshots \
                 WHERE id = $1 AND workspace_id = $2 AND project_id = $3 AND conversation_id = $4",
                &[&snapshot_id, &workspace_id, &project_id, &conversation_id],
            )
            .await?
            .ok_or(StoreError::NotFound)?;
        let boundary: i64 = snapshot.get(0);
        let current: bool = snapshot.get(1);
        if !current {
            return Err(StoreError::HistorySnapshotExpired);
        }
        let upper = if before == 0 {
            boundary.saturating_add(1)
        } else {
            before.min(boundary.saturating_add(1))
        };
        let fetch = i64::from(limit).saturating_add(1);
        let mut rows = transaction
            .query(
                "SELECT sequence, id, role, content, COALESCE(turn_id, ''), \
                        workflow_ts_seconds(accepted_at), workflow_ts_nanos(accepted_at) \
                 FROM workflow_messages \
                 WHERE conversation_id = $1 AND sequence < $2 AND sequence <= $3 \
                 ORDER BY sequence DESC LIMIT $4",
                &[&conversation_id, &upper, &boundary, &fetch],
            )
            .await?;
        let has_more =
            rows.len() > usize::try_from(limit).map_err(|_| StoreError::InvalidInput("limit"))?;
        if has_more {
            rows.pop();
        }
        let records = rows
            .iter()
            .map(history_from_message_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_before_sequence = records.last().map_or(0, |record| record.sequence);
        transaction.commit().await?;
        driver.abort();
        Ok(HistoryPage {
            records,
            snapshot_id: snapshot_id.to_owned(),
            next_before_sequence,
            has_more,
        })
    }

    /// List bounded pending outbox records in durable creation order.
    ///
    /// # Errors
    /// Returns validation, database, or retained-state errors.
    pub async fn pending_outbox(&self, limit: u32) -> Result<Vec<OutboxRecord>, StoreError> {
        if limit == 0 || limit > 1_000 {
            return Err(StoreError::InvalidInput("limit"));
        }
        let (client, driver) = crate::connect(&self.database_url).await?;
        let rows = client
            .query(
                "SELECT id,conversation_id,turn_id,subject,event_bytes FROM ( \
                 SELECT id,conversation_id,turn_id,subject,event_bytes,created_at,next_attempt_at FROM workflow_outbox \
                   WHERE state IN ('pending','stored') AND next_attempt_at <= clock_timestamp() \
                 UNION ALL SELECT decision_event_id,'','',subject,decision_bytes,created_at,next_attempt_at \
                   FROM workflow_transport_rejections WHERE state IN ('pending','stored') \
                   AND next_attempt_at <= clock_timestamp()) deliveries ORDER BY next_attempt_at,created_at LIMIT $1",
                &[&i64::from(limit)],
            )
            .await?;
        let records = rows
            .into_iter()
            .map(|row| OutboxRecord {
                id: row.get(0),
                conversation_id: row.get(1),
                turn_id: row.get(2),
                subject: row.get(3),
                event_bytes: row.get(4),
            })
            .collect();
        driver.abort();
        Ok(records)
    }
}
