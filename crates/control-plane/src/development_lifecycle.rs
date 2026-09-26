use crate::{
    development::{AgentProfile, DevelopmentPhase, DevelopmentTask},
    orchestration::{Store, StoreError},
};
use ayni_orchestrator_domain::{GitRevision, IdempotencyKey, TaskId, scope_violations_are_bounded};
use rusqlite::{OptionalExtension, params};

impl Store {
    pub fn agent_profiles(&self) -> Result<Vec<AgentProfile>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM agent_profiles ORDER BY id")
            .map_err(StoreError::Database)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter().map(|id| self.agent_profile(&id)).collect()
    }

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

    pub(crate) fn development_targets(&self) -> Result<Vec<DevelopmentTask>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM development_tasks WHERE phase IN ('queued', 'coding', 'validation_queued',
             'validating', 'pr_queued', 'preparing_pr', 'cancelled')
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

    pub(crate) fn development_phase(
        &self,
        id: TaskId,
        from: DevelopmentPhase,
        to: DevelopmentPhase,
    ) -> Result<bool, StoreError> {
        let updated = self
            .connection
            .execute(
                "UPDATE development_tasks SET phase=?1, updated_at=?2 WHERE id=?3 AND phase=?4",
                params![
                    to.as_str(),
                    super::orchestration::unix_seconds()?,
                    id.to_string(),
                    from.as_str()
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(updated == 1)
    }

    pub(crate) fn record_candidate(&self, id: TaskId, candidate: &str) -> Result<bool, StoreError> {
        GitRevision::parse(candidate).map_err(StoreError::InvalidIdentity)?;
        let updated = self.connection.execute(
            "UPDATE development_tasks SET phase='validation_queued', candidate_commit=?1, updated_at=?2
             WHERE id=?3 AND phase='coding'",
            params![candidate, super::orchestration::unix_seconds()?, id.to_string()],
        ).map_err(StoreError::Database)?;
        Ok(updated == 1)
    }

    pub(crate) fn record_validation(
        &self,
        id: TaskId,
        digest: &str,
        tests_edited: bool,
    ) -> Result<bool, StoreError> {
        let updated = self
            .connection
            .execute(
                "UPDATE development_tasks SET phase='pr_queued', validation_manifest=?1,
             policy_or_test_edits=?2, updated_at=?3 WHERE id=?4 AND phase='validating'",
                params![
                    digest,
                    tests_edited,
                    super::orchestration::unix_seconds()?,
                    id.to_string()
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(updated == 1)
    }

    pub(crate) fn fail_development(&self, id: TaskId, reason: &str) -> Result<bool, StoreError> {
        let updated = self
            .connection
            .execute(
                "UPDATE development_tasks SET phase='failed', failure=?1, updated_at=?2
             WHERE id=?3 AND phase IN ('queued','coding','validation_queued','validating',
             'pr_queued','preparing_pr')",
                params![
                    reason.chars().take(512).collect::<String>(),
                    super::orchestration::unix_seconds()?,
                    id.to_string()
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(updated == 1)
    }

    pub(crate) fn fail_validation(
        &self,
        id: TaskId,
        reason: &str,
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
        let updated = self
            .connection
            .execute(
                "UPDATE development_tasks SET phase='failed', failure=?1,
             validation_manifest=?2, policy_or_test_edits=?3, scope_violations=?4, updated_at=?5
             WHERE id=?6 AND phase='validating'",
                params![
                    reason,
                    manifest_digest,
                    policy_or_test_edits,
                    scope_violations,
                    super::orchestration::unix_seconds()?,
                    id.to_string()
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(updated == 1)
    }

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
                "SELECT task_id FROM development_actions WHERE action_key=?1",
                [key.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?;
        if let Some(existing) = existing {
            if existing != id.to_string() {
                return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
            }
            return Ok(task);
        }
        if !matches!(
            task.phase,
            DevelopmentPhase::Completed
                | DevelopmentPhase::Failed
                | DevelopmentPhase::Cancelled
                | DevelopmentPhase::Unsupported
        ) {
            transaction
                .execute(
                    "UPDATE development_tasks SET phase='cancelled', updated_at=?1 WHERE id=?2",
                    params![now, id.to_string()],
                )
                .map_err(StoreError::Database)?;
        }
        transaction
            .execute(
                "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES (?1, ?2, 'cancel', ?3)",
                params![key.as_str(), id.to_string(), now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        self.development_task(id)
    }
}
