//! Durable repository enrollment readiness and reusable environment preparation.

use crate::{
    development::RepositoryBinding,
    orchestration::{Store, StoreError, unix_seconds},
};
use ayni_orchestrator_domain::{GitRevision, IdempotencyKey};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PrepareRepositoryInput {
    pub revision: String,
}

impl PrepareRepositoryInput {
    fn validate(&self) -> Result<(), StoreError> {
        GitRevision::parse(&self.revision).map_err(StoreError::InvalidIdentity)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationStatus {
    Queued,
    Preparing,
    Ready,
    SetupRequired,
    Failed,
}

impl PreparationStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Ready => "ready",
            Self::SetupRequired => "setup_required",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "queued" => Ok(Self::Queued),
            "preparing" => Ok(Self::Preparing),
            "ready" => Ok(Self::Ready),
            "setup_required" => Ok(Self::SetupRequired),
            "failed" => Ok(Self::Failed),
            _ => Err(StoreError::CorruptState(
                "invalid persisted preparation status",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparationView {
    pub id: String,
    pub repository_id: String,
    pub revision: String,
    pub lock_sha256: String,
    pub status: PreparationStatus,
    pub diagnostic: Option<String>,
    pub duration_seconds: Option<u64>,
    pub reuse_count: u32,
    pub reused: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryView {
    #[serde(flatten)]
    pub binding: RepositoryBinding,
    pub latest_preparation: Option<PreparationView>,
}

impl Store {
    pub(crate) fn migrate_preparations(&self) -> Result<(), StoreError> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS repository_preparations (
                    id TEXT PRIMARY KEY, repository_id TEXT NOT NULL REFERENCES repository_bindings(id),
                    revision TEXT NOT NULL, lock_sha256 TEXT NOT NULL, status TEXT NOT NULL,
                    diagnostic TEXT, duration_seconds INTEGER, reuse_count INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    UNIQUE(repository_id, revision, lock_sha256)
                );
                CREATE TABLE IF NOT EXISTS preparation_actions (
                    action_key TEXT PRIMARY KEY, preparation_id TEXT NOT NULL REFERENCES repository_preparations(id),
                    repository_id TEXT NOT NULL, revision TEXT NOT NULL, reused INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS repository_preparations_status_idx
                    ON repository_preparations(status, updated_at);",
            )
            .map_err(StoreError::Database)?;
        let has_reused: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('preparation_actions') WHERE name='reused')",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::Database)?;
        if !has_reused {
            self.connection
                .execute(
                    "ALTER TABLE preparation_actions ADD COLUMN reused INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(StoreError::Database)?;
        }
        Ok(())
    }

    pub fn repository_views(&self) -> Result<Vec<RepositoryView>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM repository_bindings ORDER BY id")
            .map_err(StoreError::Database)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter()
            .map(|id| self.repository_view(&id))
            .collect()
    }

    pub fn preparations(&self, repository_id: &str) -> Result<Vec<PreparationView>, StoreError> {
        self.repository_binding(repository_id)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id FROM repository_preparations WHERE repository_id=?1
                 ORDER BY created_at DESC, id DESC",
            )
            .map_err(StoreError::Database)?;
        let ids = statement
            .query_map([repository_id], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter().map(|id| self.preparation(&id)).collect()
    }

    pub fn repository_view(&self, id: &str) -> Result<RepositoryView, StoreError> {
        let binding = self.repository_binding(id)?;
        let preparation_id = self
            .connection
            .query_row(
                "SELECT id FROM repository_preparations
                 WHERE repository_id=?1 AND lock_sha256=?2
                 ORDER BY updated_at DESC LIMIT 1",
                params![id, &binding.lock_sha256],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?;
        let latest_preparation = preparation_id
            .map(|preparation_id| self.preparation(&preparation_id))
            .transpose()?;
        Ok(RepositoryView {
            binding,
            latest_preparation,
        })
    }

    pub fn prepare_repository(
        &mut self,
        repository_id: &str,
        key: &IdempotencyKey,
        input: &PrepareRepositoryInput,
    ) -> Result<PreparationView, StoreError> {
        input.validate()?;
        if let Some(existing) = self.preparation_action(key)? {
            if existing.0 != repository_id || existing.1 != input.revision {
                return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
            }
            let mut preparation = self.preparation(&existing.2)?;
            preparation.reused = existing.3;
            return Ok(preparation);
        }
        let repository = self.repository_binding(repository_id)?;
        let id = preparation_identity(&repository, &input.revision);
        let now = unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO repository_preparations
                 (id, repository_id, revision, lock_sha256, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?5)",
                params![
                    id,
                    repository_id,
                    input.revision,
                    repository.lock_sha256,
                    now
                ],
            )
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "UPDATE repository_preparations SET status='queued', diagnostic=NULL,
                 duration_seconds=NULL, updated_at=?1
                 WHERE id=?2 AND status IN ('setup_required','failed')",
                params![now, id],
            )
            .map_err(StoreError::Database)?;
        let reused = transaction
            .query_row(
                "SELECT status='ready' FROM repository_preparations WHERE id=?1",
                [&id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(StoreError::Database)?;
        if reused {
            transaction
                .execute(
                    "UPDATE repository_preparations
                     SET reuse_count=reuse_count+1, updated_at=?1 WHERE id=?2",
                    params![now, id],
                )
                .map_err(StoreError::Database)?;
        }
        transaction
            .execute(
                "INSERT INTO preparation_actions
                 (action_key, preparation_id, repository_id, revision, reused, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![key.as_str(), id, repository_id, input.revision, reused, now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        let mut preparation = self.preparation(&id)?;
        preparation.reused = reused;
        Ok(preparation)
    }

    pub fn preparation(&self, id: &str) -> Result<PreparationView, StoreError> {
        self.connection
            .query_row(
                "SELECT repository_id, revision, lock_sha256, status, diagnostic,
                 duration_seconds, reuse_count FROM repository_preparations WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, u32>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(StoreError::Database)?
            .map(|row| {
                Ok::<PreparationView, StoreError>(PreparationView {
                    id: id.to_owned(),
                    repository_id: row.0,
                    revision: row.1,
                    lock_sha256: row.2,
                    status: PreparationStatus::parse(&row.3)?,
                    diagnostic: row.4,
                    duration_seconds: row.5.and_then(|value| u64::try_from(value).ok()),
                    reuse_count: row.6,
                    reused: false,
                })
            })
            .transpose()?
            .ok_or_else(|| StoreError::PreparationNotFound(id.to_owned()))
    }

    pub(crate) fn preparation_action(
        &self,
        key: &IdempotencyKey,
    ) -> Result<Option<(String, String, String, bool)>, StoreError> {
        self.connection
            .query_row(
                "SELECT repository_id, revision, preparation_id, reused
                 FROM preparation_actions WHERE action_key=?1",
                [key.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(StoreError::Database)
    }

    pub(crate) fn preparation_targets(&self) -> Result<Vec<PreparationView>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id FROM repository_preparations WHERE status IN ('queued','preparing')
                 ORDER BY created_at",
            )
            .map_err(StoreError::Database)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)?;
        ids.into_iter().map(|id| self.preparation(&id)).collect()
    }

    pub(crate) fn set_preparation_status(
        &self,
        id: &str,
        from: PreparationStatus,
        to: PreparationStatus,
    ) -> Result<bool, StoreError> {
        let changed = self
            .connection
            .execute(
                "UPDATE repository_preparations SET status=?1, diagnostic=NULL, updated_at=?2
                 WHERE id=?3 AND status=?4",
                params![to.as_str(), unix_seconds()?, id, from.as_str()],
            )
            .map_err(StoreError::Database)?;
        Ok(changed == 1)
    }

    pub(crate) fn complete_preparation(
        &self,
        id: &str,
        duration_seconds: u64,
    ) -> Result<bool, StoreError> {
        let duration_seconds = i64::try_from(duration_seconds)
            .map_err(|_| StoreError::InvalidInput("preparation duration is too large"))?;
        let changed = self
            .connection
            .execute(
                "UPDATE repository_preparations SET status='ready', duration_seconds=?1,
             diagnostic=NULL, updated_at=?2 WHERE id=?3 AND status='preparing'",
                params![duration_seconds, unix_seconds()?, id],
            )
            .map_err(StoreError::Database)?;
        Ok(changed == 1)
    }

    pub(crate) fn fail_preparation(
        &self,
        id: &str,
        status: PreparationStatus,
        diagnostic: &str,
    ) -> Result<bool, StoreError> {
        if !matches!(
            status,
            PreparationStatus::SetupRequired | PreparationStatus::Failed
        ) {
            return Err(StoreError::InvalidInput(
                "invalid preparation failure status",
            ));
        }
        let changed = self
            .connection
            .execute(
                "UPDATE repository_preparations SET status=?1, diagnostic=?2, updated_at=?3
             WHERE id=?4 AND status IN ('queued','preparing')",
                params![
                    status.as_str(),
                    crate::truncate_utf8(diagnostic, 512),
                    unix_seconds()?,
                    id
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(changed == 1)
    }

    pub(crate) fn ready_preparation(
        &self,
        repository_id: &str,
        revision: &str,
        lock_sha256: &str,
    ) -> Result<String, StoreError> {
        self.connection
            .query_row(
                "SELECT id FROM repository_preparations WHERE repository_id=?1 AND revision=?2
                 AND lock_sha256=?3 AND status='ready'",
                params![repository_id, revision, lock_sha256],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::Database)?
            .ok_or(StoreError::RepositoryNotReady)
    }
}

fn preparation_identity(repository: &RepositoryBinding, revision: &str) -> String {
    let bytes = format!(
        "v1\0{}\0{}/{}\0{}\0{}\0ayni-0.13.0",
        repository.id, repository.owner, repository.repository, revision, repository.lock_sha256
    );
    format!("prep-{:x}", Sha256::digest(bytes.as_bytes()))
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;
