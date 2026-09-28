use crate::{
    development::{
        DevelopmentLifecycleEvent, DevelopmentLifecycleOutcome, DevelopmentLifecycleStage,
        DevelopmentPhase, DevelopmentTask,
    },
    orchestration::{Store, StoreError},
};
use ayni_orchestrator_domain::{GitRevision, IdempotencyKey, TaskId, scope_violations_are_bounded};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use tracing::info;

pub fn backfill_development_lifecycle(connection: &Connection) -> Result<(), StoreError> {
    // Older databases did not retain a timeline. Record only the current snapshot and
    // leave lifecycle_complete false rather than inventing elapsed stage history.
    connection
        .execute_batch(
            "INSERT OR IGNORE INTO development_lifecycle_events
                (task_id, event_key, event_time, stage, outcome, reason_code)
             SELECT id, 'legacy_snapshot', updated_at,
                CASE phase
                    WHEN 'queued' THEN 'queue'
                    WHEN 'coding' THEN 'coding'
                    WHEN 'waiting_for_input' THEN 'coding'
                    WHEN 'validation_queued' THEN 'validation'
                    WHEN 'validating' THEN 'validation'
                    WHEN 'pr_queued' THEN 'pr_preparation'
                    WHEN 'preparing_pr' THEN 'pr_preparation'
                    WHEN 'completed' THEN 'pr_preparation'
                    WHEN 'unsupported' THEN 'queue'
                    ELSE 'unknown'
                END,
                CASE phase
                    WHEN 'queued' THEN 'queued'
                    WHEN 'coding' THEN 'started'
                    WHEN 'waiting_for_input' THEN 'blocked'
                    WHEN 'validation_queued' THEN 'queued'
                    WHEN 'validating' THEN 'started'
                    WHEN 'pr_queued' THEN 'queued'
                    WHEN 'preparing_pr' THEN 'started'
                    WHEN 'completed' THEN 'succeeded'
                    WHEN 'failed' THEN 'failed'
                    WHEN 'cancelled' THEN 'cancelled'
                    ELSE 'unsupported'
                END,
                CASE phase
                    WHEN 'unsupported' THEN 'unsupported_provider'
                    WHEN 'failed' THEN 'legacy_failure'
                    WHEN 'cancelled' THEN 'operator_cancelled'
                    ELSE NULL
                END
             FROM development_tasks WHERE lifecycle_complete=0;",
        )
        .map_err(StoreError::Database)
}

const fn stage_for_phase(phase: DevelopmentPhase) -> DevelopmentLifecycleStage {
    match phase {
        DevelopmentPhase::Queued | DevelopmentPhase::Unsupported => {
            DevelopmentLifecycleStage::Queue
        }
        DevelopmentPhase::Coding | DevelopmentPhase::WaitingForInput => {
            DevelopmentLifecycleStage::Coding
        }
        DevelopmentPhase::ValidationQueued | DevelopmentPhase::Validating => {
            DevelopmentLifecycleStage::Validation
        }
        DevelopmentPhase::PrQueued
        | DevelopmentPhase::PreparingPr
        | DevelopmentPhase::Completed => DevelopmentLifecycleStage::PrPreparation,
        DevelopmentPhase::Failed | DevelopmentPhase::Cancelled => {
            DevelopmentLifecycleStage::Unknown
        }
    }
}

fn insert_event(
    transaction: &Transaction<'_>,
    id: TaskId,
    event_key: &str,
    stage: DevelopmentLifecycleStage,
    outcome: DevelopmentLifecycleOutcome,
    at: i64,
    reason_code: Option<&str>,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO development_lifecycle_events
             (task_id, event_key, event_time, stage, outcome, reason_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.to_string(),
                event_key,
                at,
                stage.as_str(),
                outcome.as_str(),
                reason_code
            ],
        )
        .map_err(StoreError::Database)?;
    Ok(())
}

pub fn insert_phase_events(
    transaction: &Transaction<'_>,
    id: TaskId,
    phase: DevelopmentPhase,
    at: i64,
    reason_code: Option<&str>,
) -> Result<(), StoreError> {
    let events: &[(&str, DevelopmentLifecycleStage, DevelopmentLifecycleOutcome)] = match phase {
        DevelopmentPhase::Queued => &[(
            "queued",
            DevelopmentLifecycleStage::Queue,
            DevelopmentLifecycleOutcome::Queued,
        )],
        DevelopmentPhase::Coding => &[(
            "coding_started",
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleOutcome::Started,
        )],
        DevelopmentPhase::WaitingForInput => &[(
            "coding_blocked",
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleOutcome::Blocked,
        )],
        DevelopmentPhase::ValidationQueued => &[
            (
                "coding_succeeded",
                DevelopmentLifecycleStage::Coding,
                DevelopmentLifecycleOutcome::Succeeded,
            ),
            (
                "validation_queued",
                DevelopmentLifecycleStage::Validation,
                DevelopmentLifecycleOutcome::Queued,
            ),
        ],
        DevelopmentPhase::Validating => &[(
            "validation_started",
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleOutcome::Started,
        )],
        DevelopmentPhase::PrQueued => &[
            (
                "validation_succeeded",
                DevelopmentLifecycleStage::Validation,
                DevelopmentLifecycleOutcome::Succeeded,
            ),
            (
                "pr_preparation_queued",
                DevelopmentLifecycleStage::PrPreparation,
                DevelopmentLifecycleOutcome::Queued,
            ),
        ],
        DevelopmentPhase::PreparingPr => &[(
            "pr_preparation_started",
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleOutcome::Started,
        )],
        DevelopmentPhase::Completed => &[(
            "pr_preparation_succeeded",
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleOutcome::Succeeded,
        )],
        DevelopmentPhase::Unsupported => &[(
            "unsupported",
            DevelopmentLifecycleStage::Queue,
            DevelopmentLifecycleOutcome::Unsupported,
        )],
        DevelopmentPhase::Failed | DevelopmentPhase::Cancelled => {
            return Err(StoreError::InvalidInput(
                "terminal event requires its active stage",
            ));
        }
    };
    for (event_key, stage, outcome) in events {
        insert_event(
            transaction,
            id,
            event_key,
            *stage,
            *outcome,
            at,
            reason_code,
        )?;
    }
    Ok(())
}

fn insert_terminal_event(
    transaction: &Transaction<'_>,
    id: TaskId,
    phase: DevelopmentPhase,
    active_phase: DevelopmentPhase,
    at: i64,
    reason_code: &str,
) -> Result<(), StoreError> {
    let (event_key, outcome) = match phase {
        DevelopmentPhase::Failed => ("failed", DevelopmentLifecycleOutcome::Failed),
        DevelopmentPhase::Cancelled => ("cancelled", DevelopmentLifecycleOutcome::Cancelled),
        _ => return Err(StoreError::InvalidInput("invalid terminal lifecycle event")),
    };
    insert_event(
        transaction,
        id,
        event_key,
        stage_for_phase(active_phase),
        outcome,
        at,
        Some(reason_code),
    )
}

fn log_event(
    id: TaskId,
    stage: DevelopmentLifecycleStage,
    outcome: DevelopmentLifecycleOutcome,
    reason_code: Option<&str>,
) {
    info!(
        task_id = %id,
        stage = stage.as_str(),
        outcome = outcome.as_str(),
        reason_code,
        "development lifecycle transition"
    );
}

pub fn log_phase_events(id: TaskId, phase: DevelopmentPhase, reason_code: Option<&str>) {
    if phase == DevelopmentPhase::ValidationQueued {
        log_event(
            id,
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleOutcome::Succeeded,
            reason_code,
        );
        log_event(
            id,
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleOutcome::Queued,
            reason_code,
        );
        return;
    }
    if phase == DevelopmentPhase::PrQueued {
        log_event(
            id,
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleOutcome::Succeeded,
            reason_code,
        );
        log_event(
            id,
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleOutcome::Queued,
            reason_code,
        );
        return;
    }
    let (stage, outcome) = match phase {
        DevelopmentPhase::Queued => (
            DevelopmentLifecycleStage::Queue,
            DevelopmentLifecycleOutcome::Queued,
        ),
        DevelopmentPhase::Coding => (
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleOutcome::Started,
        ),
        DevelopmentPhase::WaitingForInput => (
            DevelopmentLifecycleStage::Coding,
            DevelopmentLifecycleOutcome::Blocked,
        ),
        DevelopmentPhase::ValidationQueued => (
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleOutcome::Queued,
        ),
        DevelopmentPhase::Validating => (
            DevelopmentLifecycleStage::Validation,
            DevelopmentLifecycleOutcome::Started,
        ),
        DevelopmentPhase::PrQueued => (
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleOutcome::Queued,
        ),
        DevelopmentPhase::PreparingPr => (
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleOutcome::Started,
        ),
        DevelopmentPhase::Completed => (
            DevelopmentLifecycleStage::PrPreparation,
            DevelopmentLifecycleOutcome::Succeeded,
        ),
        DevelopmentPhase::Unsupported => (
            DevelopmentLifecycleStage::Queue,
            DevelopmentLifecycleOutcome::Unsupported,
        ),
        DevelopmentPhase::Failed | DevelopmentPhase::Cancelled => return,
    };
    log_event(id, stage, outcome, reason_code);
}

fn log_terminal_event(
    id: TaskId,
    phase: DevelopmentPhase,
    active_phase: DevelopmentPhase,
    reason_code: &str,
) {
    info!(
        task_id = %id,
        stage = stage_for_phase(active_phase).as_str(),
        outcome = phase.as_str(),
        reason_code,
        "development lifecycle transition"
    );
}

impl Store {
    pub fn development_tasks(&self) -> Result<Vec<DevelopmentTask>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM development_tasks ORDER BY created_at DESC LIMIT 100")
            .map_err(StoreError::Database)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter()
            .map(|value| {
                self.development_task(
                    TaskId::parse(&value).map_err(StoreError::InvalidLifecycleData)?,
                )
            })
            .collect()
    }

    pub(crate) fn development_tasks_all(&self) -> Result<Vec<DevelopmentTask>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM development_tasks ORDER BY created_at DESC, id DESC")
            .map_err(StoreError::Database)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter()
            .map(|value| {
                self.development_task(
                    TaskId::parse(&value).map_err(StoreError::InvalidLifecycleData)?,
                )
            })
            .collect()
    }

    pub(crate) fn development_lifecycle(
        &self,
        id: TaskId,
    ) -> Result<Vec<DevelopmentLifecycleEvent>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT event_id, event_time, stage, outcome, reason_code
                 FROM development_lifecycle_events WHERE task_id=?1 ORDER BY event_id ASC",
            )
            .map_err(StoreError::Database)?;
        statement
            .query_map([id.to_string()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(StoreError::Database)?
            .map(|row| {
                let (sequence, event_time, stage, outcome, reason_code) =
                    row.map_err(StoreError::Database)?;
                Ok(DevelopmentLifecycleEvent {
                    sequence,
                    event_time,
                    stage: DevelopmentLifecycleStage::parse(&stage)?,
                    outcome: DevelopmentLifecycleOutcome::parse(&outcome)?,
                    reason_code,
                })
            })
            .collect()
    }

    pub(crate) fn development_targets(&self) -> Result<Vec<DevelopmentTask>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM development_tasks WHERE phase IN ('queued', 'coding', 'validation_queued',
             'validating', 'pr_queued', 'preparing_pr', 'cancelled')
             OR (phase IN ('completed','failed','waiting_for_input') AND EXISTS (
                SELECT 1 FROM development_model_usage_collection collection
                WHERE collection.task_id=development_tasks.id AND collection.complete=0))
             ORDER BY created_at ASC"
        ).map_err(StoreError::Database)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter()
            .map(|id| {
                self.development_task(TaskId::parse(&id).map_err(StoreError::InvalidLifecycleData)?)
            })
            .collect()
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, from = from.as_str(), to = to.as_str()))]
    pub(crate) fn development_phase(
        &mut self,
        id: TaskId,
        from: DevelopmentPhase,
        to: DevelopmentPhase,
    ) -> Result<bool, StoreError> {
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let updated = transaction
            .execute(
                "UPDATE development_tasks SET phase=?1, updated_at=?2,
                 resource_version=resource_version+1 WHERE id=?3 AND phase=?4",
                params![to.as_str(), now, id.to_string(), from.as_str()],
            )
            .map_err(StoreError::Database)?;
        if updated == 1 {
            insert_phase_events(&transaction, id, to, now, None)?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        if updated == 1 {
            log_phase_events(id, to, None);
        }
        Ok(updated == 1)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, transition = "candidate_recorded"))]
    pub(crate) fn record_candidate(
        &mut self,
        id: TaskId,
        candidate: &str,
    ) -> Result<bool, StoreError> {
        GitRevision::parse(candidate).map_err(StoreError::InvalidIdentity)?;
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let updated = transaction.execute(
            "UPDATE development_tasks SET phase='validation_queued', candidate_commit=?1, updated_at=?2,
             resource_version=resource_version+1 WHERE id=?3 AND phase='coding'",
            params![candidate, now, id.to_string()],
        ).map_err(StoreError::Database)?;
        if updated == 1 {
            insert_phase_events(
                &transaction,
                id,
                DevelopmentPhase::ValidationQueued,
                now,
                None,
            )?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        if updated == 1 {
            log_phase_events(id, DevelopmentPhase::ValidationQueued, None);
        }
        Ok(updated == 1)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, transition = "validation_recorded"))]
    pub(crate) fn record_validation(
        &mut self,
        id: TaskId,
        digest: &str,
        tests_edited: bool,
    ) -> Result<bool, StoreError> {
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let updated = transaction
            .execute(
                "UPDATE development_tasks SET phase='pr_queued', validation_manifest=?1,
             policy_or_test_edits=?2, updated_at=?3, resource_version=resource_version+1
             WHERE id=?4 AND phase='validating'",
                params![digest, tests_edited, now, id.to_string()],
            )
            .map_err(StoreError::Database)?;
        if updated == 1 {
            insert_phase_events(&transaction, id, DevelopmentPhase::PrQueued, now, None)?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        if updated == 1 {
            log_phase_events(id, DevelopmentPhase::PrQueued, None);
        }
        Ok(updated == 1)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, reason_code))]
    pub(crate) fn fail_development(
        &mut self,
        id: TaskId,
        reason_code: &str,
        diagnostic: &str,
    ) -> Result<bool, StoreError> {
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let current: String = transaction
            .query_row(
                "SELECT phase FROM development_tasks WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::Database)?;
        let active_phase = DevelopmentPhase::parse(&current)?;
        let updated = transaction
            .execute(
                "UPDATE development_tasks SET phase='failed', failure=?1, updated_at=?2,
             resource_version=resource_version+1
             WHERE id=?3 AND phase IN ('queued','coding','validation_queued','validating',
             'pr_queued','preparing_pr')",
                params![crate::truncate_utf8(diagnostic, 512), now, id.to_string()],
            )
            .map_err(StoreError::Database)?;
        if updated == 1 {
            insert_terminal_event(
                &transaction,
                id,
                DevelopmentPhase::Failed,
                active_phase,
                now,
                reason_code,
            )?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        if updated == 1 {
            log_terminal_event(id, DevelopmentPhase::Failed, active_phase, reason_code);
        }
        Ok(updated == 1)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, reason_code))]
    pub(crate) fn fail_validation(
        &mut self,
        id: TaskId,
        reason_code: &str,
        diagnostic: &str,
        manifest_digest: Option<&str>,
        policy_or_test_edits: Option<bool>,
        scope_violations: &[String],
    ) -> Result<bool, StoreError> {
        if !scope_violations_are_bounded(scope_violations) {
            return Err(StoreError::InvalidInput(
                "scope violations exceed durable storage limits",
            ));
        }
        let scope_violations = serde_json::to_string(scope_violations)
            .map_err(|_| StoreError::InvalidInput("could not serialize scope violations"))?;
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let updated = transaction
            .execute(
                "UPDATE development_tasks SET phase='failed', failure=?1,
             validation_manifest=?2, policy_or_test_edits=?3, scope_violations=?4, updated_at=?5,
             resource_version=resource_version+1 WHERE id=?6 AND phase='validating'",
                params![
                    diagnostic,
                    manifest_digest,
                    policy_or_test_edits,
                    scope_violations,
                    now,
                    id.to_string()
                ],
            )
            .map_err(StoreError::Database)?;
        if updated == 1 {
            insert_terminal_event(
                &transaction,
                id,
                DevelopmentPhase::Failed,
                DevelopmentPhase::Validating,
                now,
                reason_code,
            )?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        if updated == 1 {
            log_terminal_event(
                id,
                DevelopmentPhase::Failed,
                DevelopmentPhase::Validating,
                reason_code,
            );
        }
        Ok(updated == 1)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, transition = "cancel"))]
    pub fn cancel_development(
        &mut self,
        id: TaskId,
        key: &IdempotencyKey,
    ) -> Result<DevelopmentTask, StoreError> {
        let task = self.development_task(id)?;
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let existing = transaction
            .query_row(
                "SELECT task_id, action_kind FROM development_actions WHERE action_key=?1",
                [key.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?;
        if let Some((existing, action_kind)) = existing {
            if existing != id.to_string() || action_kind != "cancel" {
                return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
            }
            return Ok(task);
        }
        let cancelled = if matches!(
            task.phase,
            DevelopmentPhase::Completed
                | DevelopmentPhase::Failed
                | DevelopmentPhase::Cancelled
                | DevelopmentPhase::Unsupported
        ) {
            false
        } else {
            transaction
                .execute(
                    "UPDATE development_tasks SET phase='cancelled', updated_at=?1,
                     resource_version=resource_version+1 WHERE id=?2",
                    params![now, id.to_string()],
                )
                .map_err(StoreError::Database)?;
            insert_terminal_event(
                &transaction,
                id,
                DevelopmentPhase::Cancelled,
                task.phase,
                now,
                "operator_cancelled",
            )?;
            true
        };
        transaction
            .execute(
                "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES (?1, ?2, 'cancel', ?3)",
                params![key.as_str(), id.to_string(), now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        if cancelled {
            log_terminal_event(
                id,
                DevelopmentPhase::Cancelled,
                task.phase,
                "operator_cancelled",
            );
        }
        self.development_task(id)
    }
}
