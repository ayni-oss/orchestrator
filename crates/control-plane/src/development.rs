//! Durable M3 request contracts. Execution is activated only after the selected
//! repository, agent profile, and credential references are configured.

use ayni_orchestrator_domain::{GitRevision, IdempotencyKey, Sha256Digest, TaskId};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::orchestration::{Store, StoreError};

const SUPPORTED_REPOSITORY: &str = "ayni-oss/orchestrator";

/// One explicitly granted GitHub repository.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryBinding {
    pub id: String,
    pub owner: String,
    pub repository: String,
    pub installation_id: i64,
    pub base_branch: String,
    pub lock_sha256: String,
}

impl RepositoryBinding {
    fn validate(&self) -> Result<(), StoreError> {
        if self.id.is_empty() || self.id.len() > 64 || !slug(&self.id) {
            return Err(StoreError::InvalidInput(
                "repository id must be a 1-64 character slug",
            ));
        }
        if format!("{}/{}", self.owner, self.repository) != SUPPORTED_REPOSITORY {
            return Err(StoreError::InvalidInput(
                "M3 supports only ayni-oss/orchestrator",
            ));
        }
        if self.installation_id <= 0 || !git_branch(&self.base_branch) {
            return Err(StoreError::InvalidInput(
                "invalid GitHub installation or base branch",
            ));
        }
        Sha256Digest::parse(&self.lock_sha256).map_err(StoreError::InvalidIdentity)?;
        Ok(())
    }
}

/// Provider-neutral model selection. M3 execution supports `OpenAI` only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfile {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub secret_ref: String,
}

impl AgentProfile {
    fn validate(&self) -> Result<(), StoreError> {
        if self.id.is_empty() || self.id.len() > 64 || !slug(&self.id) {
            return Err(StoreError::InvalidInput(
                "agent profile id must be a 1-64 character slug",
            ));
        }
        if self.provider.is_empty() || self.provider.len() > 64 || !slug(&self.provider) {
            return Err(StoreError::InvalidInput("invalid provider"));
        }
        if self.model.is_empty() || self.model.len() > 128 || !model_text(&self.model) {
            return Err(StoreError::InvalidInput("invalid model"));
        }
        if self.secret_ref.is_empty() || self.secret_ref.len() > 128 || !slug(&self.secret_ref) {
            return Err(StoreError::InvalidInput("invalid secret reference"));
        }
        Ok(())
    }
}

/// The immutable development request accepted by the M3 API.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentInput {
    pub repository_id: String,
    pub profile_id: String,
    pub start_revision: String,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
}

impl DevelopmentInput {
    fn validate(&self) -> Result<(), StoreError> {
        if !slug(&self.repository_id) || !slug(&self.profile_id) {
            return Err(StoreError::InvalidInput("invalid repository or profile id"));
        }
        GitRevision::parse(&self.start_revision).map_err(StoreError::InvalidIdentity)?;
        if self.request.trim().is_empty() || self.request.len() > 16_384 {
            return Err(StoreError::InvalidInput(
                "request must be 1-16384 characters",
            ));
        }
        if self.acceptance_criteria.is_empty()
            || self.acceptance_criteria.len() > 20
            || self
                .acceptance_criteria
                .iter()
                .any(|criterion| criterion.trim().is_empty() || criterion.len() > 2048)
        {
            return Err(StoreError::InvalidInput(
                "provide 1-20 bounded acceptance criteria",
            ));
        }
        Ok(())
    }
}

/// A single durable M3 request and its measured candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DevelopmentTask {
    pub id: TaskId,
    pub input: DevelopmentInput,
    pub repository: RepositoryBinding,
    pub agent_profile: AgentProfile,
    pub phase: DevelopmentPhase,
    pub candidate_commit: Option<String>,
    pub validation_manifest: Option<String>,
    pub policy_or_test_edits: Option<bool>,
    pub draft_pr_url: Option<String>,
    pub failure: Option<String>,
}

pub struct PublicationAction {
    pub candidate_commit: String,
    pub title: String,
    pub body: String,
    pub completed_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentPhase {
    Queued,
    Coding,
    ValidationQueued,
    Validating,
    PrQueued,
    PreparingPr,
    Completed,
    Failed,
    Cancelled,
    Unsupported,
}

impl DevelopmentPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Coding => "coding",
            Self::ValidationQueued => "validation_queued",
            Self::Validating => "validating",
            Self::PrQueued => "pr_queued",
            Self::PreparingPr => "preparing_pr",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unsupported => "unsupported",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "queued" => Ok(Self::Queued),
            "coding" => Ok(Self::Coding),
            "validation_queued" => Ok(Self::ValidationQueued),
            "validating" => Ok(Self::Validating),
            "pr_queued" => Ok(Self::PrQueued),
            "preparing_pr" => Ok(Self::PreparingPr),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "unsupported" => Ok(Self::Unsupported),
            _ => Err(StoreError::InvalidInput(
                "invalid persisted development phase",
            )),
        }
    }
}

include!("development_validation.rs");

impl Store {
    pub(crate) fn migrate_development(&self) -> Result<(), StoreError> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS repository_bindings (
                id TEXT PRIMARY KEY, owner TEXT NOT NULL, repository TEXT NOT NULL,
                installation_id INTEGER NOT NULL, base_branch TEXT NOT NULL,
                lock_sha256 TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_profiles (
                id TEXT PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL,
                secret_ref TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS development_tasks (
                id TEXT PRIMARY KEY, repository_id TEXT NOT NULL REFERENCES repository_bindings(id),
                profile_id TEXT NOT NULL REFERENCES agent_profiles(id),
                repository_snapshot TEXT NOT NULL, profile_snapshot TEXT NOT NULL,
                start_revision TEXT NOT NULL, request TEXT NOT NULL,
                acceptance_criteria TEXT NOT NULL, phase TEXT NOT NULL,
                candidate_commit TEXT, validation_manifest TEXT,
                policy_or_test_edits INTEGER, draft_pr_url TEXT, failure TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS development_actions (
                action_key TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES development_tasks(id),
                action_kind TEXT NOT NULL, created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS publication_actions (
                task_id TEXT PRIMARY KEY REFERENCES development_tasks(id),
                branch_name TEXT NOT NULL UNIQUE, candidate_commit TEXT NOT NULL,
                status TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL,
                pr_number INTEGER, pr_url TEXT,
                updated_at INTEGER NOT NULL
            );",
            )
            .map_err(StoreError::Database)
    }

    pub fn put_repository_binding(&self, binding: &RepositoryBinding) -> Result<(), StoreError> {
        binding.validate()?;
        self.connection.execute(
            "INSERT INTO repository_bindings(id, owner, repository, installation_id, base_branch, lock_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET owner=excluded.owner, repository=excluded.repository,
             installation_id=excluded.installation_id, base_branch=excluded.base_branch,
             lock_sha256=excluded.lock_sha256",
            params![binding.id, binding.owner, binding.repository,
                binding.installation_id, binding.base_branch, binding.lock_sha256],
        ).map_err(StoreError::Database)?;
        Ok(())
    }

    pub fn put_agent_profile(&self, profile: &AgentProfile) -> Result<(), StoreError> {
        profile.validate()?;
        self.connection
            .execute(
                "INSERT INTO agent_profiles(id, provider, model, secret_ref)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET provider=excluded.provider, model=excluded.model,
             secret_ref=excluded.secret_ref",
                params![
                    profile.id,
                    profile.provider,
                    profile.model,
                    profile.secret_ref
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(())
    }

    pub fn submit_development(
        &mut self,
        key: &IdempotencyKey,
        input: &DevelopmentInput,
    ) -> Result<DevelopmentTask, StoreError> {
        input.validate()?;
        if let Some(existing) = self.existing_development_submission(key, input)? {
            return Ok(existing);
        }
        let repository = self.repository_binding(&input.repository_id)?;
        let profile = self.agent_profile(&input.profile_id)?;
        let id = TaskId::generate();
        let phase = initial_phase(&profile.provider);
        let criteria = serde_json::to_string(&input.acceptance_criteria)
            .map_err(|_| StoreError::InvalidInput("could not serialize acceptance criteria"))?;
        let repository_snapshot = serde_json::to_string(&repository)
            .map_err(|_| StoreError::InvalidInput("could not serialize repository"))?;
        let profile_snapshot = serde_json::to_string(&profile)
            .map_err(|_| StoreError::InvalidInput("could not serialize profile"))?;
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        transaction.execute(
            "INSERT INTO development_tasks(id, repository_id, profile_id, repository_snapshot, profile_snapshot, start_revision, request,
             acceptance_criteria, phase, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![id.to_string(), input.repository_id, input.profile_id, repository_snapshot,
                profile_snapshot, input.start_revision, input.request, criteria, phase.as_str(), now],
        ).map_err(StoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES (?1, ?2, 'submit', ?3)",
                params![key.as_str(), id.to_string(), now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        self.development_task(id)
    }

    fn existing_development_submission(
        &self,
        key: &IdempotencyKey,
        input: &DevelopmentInput,
    ) -> Result<Option<DevelopmentTask>, StoreError> {
        let Some(id) = self
            .connection
            .query_row(
                "SELECT task_id FROM development_actions WHERE action_key=?1",
                [key.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?
        else {
            return Ok(None);
        };
        let existing =
            self.development_task(TaskId::parse(&id).map_err(StoreError::InvalidLifecycleData)?)?;
        if existing.input != *input {
            return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
        }
        Ok(Some(existing))
    }

    fn repository_binding(&self, id: &str) -> Result<RepositoryBinding, StoreError> {
        self.connection.query_row(
            "SELECT owner, repository, installation_id, base_branch, lock_sha256 FROM repository_bindings WHERE id=?1",
            [id],
            |row| Ok(RepositoryBinding {
                id: id.to_owned(), owner: row.get(0)?, repository: row.get(1)?,
                installation_id: row.get(2)?, base_branch: row.get(3)?, lock_sha256: row.get(4)?,
            }),
        ).optional().map_err(StoreError::Database)?
            .ok_or(StoreError::InvalidInput("repository binding does not exist"))
    }

    fn agent_profile(&self, id: &str) -> Result<AgentProfile, StoreError> {
        self.connection
            .query_row(
                "SELECT provider, model, secret_ref FROM agent_profiles WHERE id=?1",
                [id],
                |row| {
                    Ok(AgentProfile {
                        id: id.to_owned(),
                        provider: row.get(0)?,
                        model: row.get(1)?,
                        secret_ref: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::Database)?
            .ok_or(StoreError::InvalidInput("agent profile does not exist"))
    }

    pub fn development_task(&self, id: TaskId) -> Result<DevelopmentTask, StoreError> {
        let row = self.connection.query_row(
            "SELECT repository_id, profile_id, repository_snapshot, profile_snapshot, start_revision, request, acceptance_criteria,
             phase, candidate_commit, validation_manifest, policy_or_test_edits, draft_pr_url, failure
             FROM development_tasks WHERE id=?1", [id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?, row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<bool>>(10)?, row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?))
            }
        ).optional().map_err(StoreError::Database)?.ok_or(StoreError::TaskNotFound(id))?;
        let repository = serde_json::from_str(&row.2)
            .map_err(|_| StoreError::InvalidInput("invalid persisted repository"))?;
        let agent_profile = serde_json::from_str(&row.3)
            .map_err(|_| StoreError::InvalidInput("invalid persisted profile"))?;
        let criteria = serde_json::from_str(&row.6)
            .map_err(|_| StoreError::InvalidInput("invalid persisted acceptance criteria"))?;
        Ok(DevelopmentTask {
            id,
            input: DevelopmentInput {
                repository_id: row.0,
                profile_id: row.1,
                start_revision: row.4,
                request: row.5,
                acceptance_criteria: criteria,
            },
            repository,
            agent_profile,
            phase: DevelopmentPhase::parse(&row.7)?,
            candidate_commit: row.8,
            validation_manifest: row.9,
            policy_or_test_edits: row.10,
            draft_pr_url: row.11,
            failure: row.12,
        })
    }

    pub(crate) fn begin_publication(
        &self,
        id: TaskId,
        title: &str,
        body: &str,
    ) -> Result<(DevelopmentTask, PublicationAction), StoreError> {
        let task = self.development_task(id)?;
        if task.phase != DevelopmentPhase::PreparingPr {
            if task.phase == DevelopmentPhase::Completed {
                let action = self.publication_action(id)?;
                return Ok((task, action));
            }
            return Err(StoreError::InvalidInput(
                "task is not ready for publication",
            ));
        }
        let candidate = task
            .candidate_commit
            .as_deref()
            .ok_or(StoreError::InvalidInput("task has no frozen candidate"))?;
        if task.validation_manifest.is_none() {
            return Err(StoreError::InvalidInput("task has no final validation"));
        }
        let branch = format!("m3/task-{}", task.id);
        self.connection
            .execute(
                "INSERT OR IGNORE INTO publication_actions(task_id, branch_name, candidate_commit,
             status, title, body, updated_at) VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
                params![
                    id.to_string(),
                    branch,
                    candidate,
                    title,
                    body,
                    super::orchestration::unix_seconds()?
                ],
            )
            .map_err(StoreError::Database)?;
        let action = self.publication_action(id)?;
        if action.candidate_commit != candidate {
            return Err(StoreError::InvalidInput(
                "publication action binds another candidate",
            ));
        }
        Ok((task, action))
    }

    pub(crate) fn publication_action(&self, id: TaskId) -> Result<PublicationAction, StoreError> {
        self.connection
            .query_row(
            "SELECT candidate_commit, title, body, pr_url FROM publication_actions WHERE task_id=?1",
                [id.to_string()],
                |row| {
                    Ok(PublicationAction {
                candidate_commit: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
                completed_url: row.get(3)?,
                    })
                },
            )
            .map_err(StoreError::Database)
    }

    pub(crate) fn complete_publication(
        &mut self,
        id: TaskId,
        candidate: &str,
        number: u64,
        url: &str,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let changed = transaction
            .execute(
                "UPDATE publication_actions SET status='completed', pr_number=?1, pr_url=?2,
             updated_at=?3 WHERE task_id=?4 AND candidate_commit=?5",
                params![
                    i64::try_from(number)
                        .map_err(|_| StoreError::InvalidInput("PR number too large"))?,
                    url,
                    super::orchestration::unix_seconds()?,
                    id.to_string(),
                    candidate
                ],
            )
            .map_err(StoreError::Database)?;
        if changed != 1 {
            return Err(StoreError::InvalidInput("publication candidate changed"));
        }
        let updated = transaction
            .execute(
                "UPDATE development_tasks SET phase='completed', draft_pr_url=?1,
             updated_at=?2 WHERE id=?3 AND phase='preparing_pr' AND candidate_commit=?4",
                params![
                    url,
                    super::orchestration::unix_seconds()?,
                    id.to_string(),
                    candidate
                ],
            )
            .map_err(StoreError::Database)?;
        if updated != 1 {
            return Err(StoreError::InvalidInput(
                "task changed before PR completion",
            ));
        }
        transaction.commit().map_err(StoreError::Database)
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
    ) -> Result<bool, StoreError> {
        let updated = self
            .connection
            .execute(
                "UPDATE development_tasks SET phase='failed', failure=?1,
             validation_manifest=?2, policy_or_test_edits=?3, updated_at=?4
             WHERE id=?5 AND phase='validating'",
                params![
                    reason,
                    manifest_digest,
                    policy_or_test_edits,
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

#[cfg(test)]
#[path = "development_tests.rs"]
mod tests;
