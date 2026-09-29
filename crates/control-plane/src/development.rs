//! Durable development request contracts. Execution is activated only after the selected
//! repository, agent profile, and credential references are configured.

use ayni_orchestrator_domain::{
    ChangeConstraints, GitRevision, IdempotencyKey, Sha256Digest, TaskId,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{
    model_usage::TaskModelUsage,
    orchestration::{Store, StoreError},
};

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
    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        if self.id.is_empty() || self.id.len() > 64 || !slug(&self.id) {
            return Err(StoreError::InvalidInput(
                "repository id must be a 1-64 character slug",
            ));
        }
        if !github_name(&self.owner) || !github_name(&self.repository) {
            return Err(StoreError::InvalidInput(
                "repository owner and name must be valid GitHub names",
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

/// Provider-neutral model selection. Development execution supports `OpenAI` only.
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

/// The immutable development request accepted by the API.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentInput {
    pub repository_id: String,
    pub profile_id: String,
    pub start_revision: String,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub change_constraints: ChangeConstraints,
    #[serde(default)]
    pub deadline_seconds: Option<u64>,
}

impl DevelopmentInput {
    pub(crate) fn validate(&self) -> Result<(), StoreError> {
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
        self.change_constraints
            .validate()
            .map_err(StoreError::InvalidChangeConstraints)?;
        if matches!(self.deadline_seconds, Some(0 | 86_401..)) {
            return Err(StoreError::InvalidInput(
                "deadline seconds must be between 1 and 86400",
            ));
        }
        Ok(())
    }
}

fn persisted_deadline_seconds(value: Option<u64>) -> Result<Option<i64>, StoreError> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| StoreError::InvalidInput("deadline seconds exceed storage bounds"))
}

fn insert_model_usage_collection(
    transaction: &rusqlite::Transaction<'_>,
    id: TaskId,
    complete: bool,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "INSERT INTO development_model_usage_collection(task_id, complete) VALUES (?1, ?2)",
            params![id.to_string(), complete],
        )
        .map_err(StoreError::Database)?;
    Ok(())
}

fn retry_request(
    source: &DevelopmentTask,
    provided_input: Option<&str>,
) -> Result<String, StoreError> {
    let allowed = match provided_input {
        Some(value) => {
            if value.trim().is_empty() || value.len() > 8_192 {
                return Err(StoreError::InvalidInput(
                    "provided input must be 1-8192 characters",
                ));
            }
            source.phase == DevelopmentPhase::WaitingForInput
        }
        None => matches!(
            source.phase,
            DevelopmentPhase::Failed | DevelopmentPhase::Cancelled
        ),
    };
    if !allowed {
        return Err(StoreError::TaskPhaseNotEligible);
    }
    Ok(source.input.request.clone())
}

/// A measurable stage in the development pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentLifecycleStage {
    Queue,
    Coding,
    Validation,
    PrPreparation,
    Unknown,
}

impl DevelopmentLifecycleStage {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Coding => "coding",
            Self::Validation => "validation",
            Self::PrPreparation => "pr_preparation",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "queue" => Ok(Self::Queue),
            "coding" => Ok(Self::Coding),
            "validation" => Ok(Self::Validation),
            "pr_preparation" => Ok(Self::PrPreparation),
            "unknown" => Ok(Self::Unknown),
            _ => Err(StoreError::CorruptState(
                "invalid persisted development lifecycle stage",
            )),
        }
    }
}

/// The bounded result recorded at a lifecycle boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentLifecycleOutcome {
    Queued,
    Started,
    Succeeded,
    Blocked,
    Failed,
    Cancelled,
    Unsupported,
}

impl DevelopmentLifecycleOutcome {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unsupported => "unsupported",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "queued" => Ok(Self::Queued),
            "started" => Ok(Self::Started),
            "succeeded" => Ok(Self::Succeeded),
            "blocked" => Ok(Self::Blocked),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "unsupported" => Ok(Self::Unsupported),
            _ => Err(StoreError::CorruptState(
                "invalid persisted development lifecycle outcome",
            )),
        }
    }
}

/// One append-only, idempotent development lifecycle event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DevelopmentLifecycleEvent {
    pub sequence: i64,
    pub event_time: i64,
    pub stage: DevelopmentLifecycleStage,
    pub outcome: DevelopmentLifecycleOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

/// A single durable development request and its measured candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DevelopmentTask {
    pub id: TaskId,
    pub input: DevelopmentInput,
    pub repository: RepositoryBinding,
    pub agent_profile: AgentProfile,
    pub preparation_id: Option<String>,
    pub retry_of: Option<TaskId>,
    #[serde(skip_serializing)]
    pub provided_input: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deadline_at: Option<i64>,
    pub phase: DevelopmentPhase,
    pub requested_input: Option<String>,
    pub candidate_commit: Option<String>,
    pub validation_manifest: Option<String>,
    pub policy_or_test_edits: Option<bool>,
    pub scope_violations: Vec<String>,
    pub draft_pr_url: Option<String>,
    pub failure: Option<String>,
    pub lifecycle_complete: bool,
    pub lifecycle: Vec<DevelopmentLifecycleEvent>,
    pub model_usage: TaskModelUsage,
    /// Monotonically increasing cursor for every durable resource change.
    #[serde(skip_serializing)]
    pub resource_version: u64,
}

impl DevelopmentTask {
    pub(crate) fn execution_request(&self) -> String {
        self.provided_input.as_ref().map_or_else(
            || self.input.request.clone(),
            |input| format!("{}\n\nMaintainer input:\n{input}", self.input.request),
        )
    }
}

pub struct PublicationAction {
    pub branch_name: String,
    pub candidate_commit: String,
    pub title: String,
    pub body: String,
    pub completed_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentPhase {
    Queued,
    Coding,
    WaitingForInput,
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
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Coding => "coding",
            Self::WaitingForInput => "waiting_for_input",
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

    pub(crate) fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "queued" => Ok(Self::Queued),
            "coding" => Ok(Self::Coding),
            "waiting_for_input" => Ok(Self::WaitingForInput),
            "validation_queued" => Ok(Self::ValidationQueued),
            "validating" => Ok(Self::Validating),
            "pr_queued" => Ok(Self::PrQueued),
            "preparing_pr" => Ok(Self::PreparingPr),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "unsupported" => Ok(Self::Unsupported),
            _ => Err(StoreError::CorruptState(
                "invalid persisted development phase",
            )),
        }
    }
}

include!("development_validation.rs");
include!("development_application.rs");

include!("development_migration.rs");

impl Store {
    #[tracing::instrument(
        skip_all,
        fields(repository_id = %input.repository_id, profile_id = %input.profile_id)
    )]
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
        let phase = initial_phase(&profile.provider);
        let preparation_id = if phase == DevelopmentPhase::Queued {
            Some(self.ready_preparation(
                &input.repository_id,
                &input.start_revision,
                &repository.lock_sha256,
            )?)
        } else {
            None
        };
        let id = TaskId::generate();
        let criteria = serde_json::to_string(&input.acceptance_criteria)
            .map_err(|_| StoreError::InvalidInput("could not serialize acceptance criteria"))?;
        let protected_paths = serde_json::to_string(&input.change_constraints.protected_paths)
            .map_err(|_| StoreError::InvalidInput("could not serialize protected paths"))?;
        let repository_snapshot = serde_json::to_string(&repository)
            .map_err(|_| StoreError::InvalidInput("could not serialize repository"))?;
        let profile_snapshot = serde_json::to_string(&profile)
            .map_err(|_| StoreError::InvalidInput("could not serialize profile"))?;
        let now = super::orchestration::unix_seconds()?;
        let deadline_seconds = persisted_deadline_seconds(input.deadline_seconds)?;
        let deadline_at = deadline_seconds.map(|seconds| now + seconds);
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        transaction.execute(
            "INSERT INTO development_tasks(id, repository_id, profile_id, repository_snapshot, profile_snapshot, start_revision, request,
             acceptance_criteria, protected_paths, allow_test_edits, phase, preparation_id,
             created_at, updated_at, deadline_seconds, deadline_at, lifecycle_complete)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14, ?15, 1)",
            params![id.to_string(), input.repository_id, input.profile_id, repository_snapshot,
                profile_snapshot, input.start_revision, input.request, criteria, protected_paths,
                input.change_constraints.allow_test_edits, phase.as_str(), preparation_id, now,
                deadline_seconds, deadline_at],
        ).map_err(StoreError::Database)?;
        insert_model_usage_collection(&transaction, id, phase == DevelopmentPhase::Unsupported)?;
        super::development_lifecycle::insert_phase_events(
            &transaction,
            id,
            phase,
            now,
            (phase == DevelopmentPhase::Unsupported).then_some("unsupported_provider"),
        )?;
        transaction
            .execute(
                "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES (?1, ?2, 'submit', ?3)",
                params![key.as_str(), id.to_string(), now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        super::development_lifecycle::log_phase_events(
            id,
            phase,
            (phase == DevelopmentPhase::Unsupported).then_some("unsupported_provider"),
        );
        self.development_task(id)
    }

    pub(crate) fn existing_development_submission(
        &self,
        key: &IdempotencyKey,
        input: &DevelopmentInput,
    ) -> Result<Option<DevelopmentTask>, StoreError> {
        let Some((id, action_kind)) = self
            .connection
            .query_row(
                "SELECT task_id, action_kind FROM development_actions WHERE action_key=?1",
                [key.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?
        else {
            return Ok(None);
        };
        let existing =
            self.development_task(TaskId::parse(&id).map_err(StoreError::InvalidLifecycleData)?)?;
        if action_kind != "submit" || existing.input != *input {
            return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
        }
        Ok(Some(existing))
    }

    pub(crate) fn repository_binding(&self, id: &str) -> Result<RepositoryBinding, StoreError> {
        self.connection.query_row(
            "SELECT owner, repository, installation_id, base_branch, lock_sha256 FROM repository_bindings WHERE id=?1",
            [id],
            |row| Ok(RepositoryBinding {
                id: id.to_owned(), owner: row.get(0)?, repository: row.get(1)?,
                installation_id: row.get(2)?, base_branch: row.get(3)?, lock_sha256: row.get(4)?,
            }),
        ).optional().map_err(StoreError::Database)?
            .ok_or_else(|| StoreError::RepositoryNotFound(id.to_owned()))
    }

    pub(crate) fn agent_profile(&self, id: &str) -> Result<AgentProfile, StoreError> {
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
             protected_paths, allow_test_edits, phase, preparation_id, retry_of, provided_input, requested_input,
             candidate_commit, validation_manifest, policy_or_test_edits, scope_violations,
             draft_pr_url, failure, created_at, updated_at, deadline_seconds, deadline_at,
             lifecycle_complete, resource_version FROM development_tasks WHERE id=?1", [id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?,
                    row.get::<_, bool>(8)?, row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?, row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?, row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?, row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<bool>>(16)?, row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?, row.get::<_, Option<String>>(19)?,
                    row.get::<_, i64>(20)?, row.get::<_, i64>(21)?,
                    row.get::<_, Option<i64>>(22)?, row.get::<_, Option<i64>>(23)?,
                    row.get::<_, bool>(24)?, row.get::<_, i64>(25)?))
            }
        ).optional().map_err(StoreError::Database)?.ok_or(StoreError::TaskNotFound(id))?;
        let repository = serde_json::from_str(&row.2)
            .map_err(|_| StoreError::CorruptState("invalid persisted repository"))?;
        let agent_profile = serde_json::from_str(&row.3)
            .map_err(|_| StoreError::CorruptState("invalid persisted profile"))?;
        let criteria = serde_json::from_str(&row.6)
            .map_err(|_| StoreError::CorruptState("invalid persisted acceptance criteria"))?;
        let protected_paths = serde_json::from_str(&row.7)
            .map_err(|_| StoreError::CorruptState("invalid persisted protected paths"))?;
        let scope_violations = row
            .17
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| StoreError::CorruptState("invalid persisted scope violations"))?
            .unwrap_or_default();
        let deadline_seconds = row
            .22
            .map(|seconds| {
                u64::try_from(seconds)
                    .map_err(|_| StoreError::CorruptState("invalid persisted task deadline"))
            })
            .transpose()?;
        let lifecycle = self.development_lifecycle(id)?;
        let model_usage = self.task_model_usage(id)?;
        Ok(DevelopmentTask {
            id,
            input: DevelopmentInput {
                repository_id: row.0,
                profile_id: row.1,
                start_revision: row.4,
                request: row.5,
                acceptance_criteria: criteria,
                change_constraints: ChangeConstraints {
                    protected_paths,
                    allow_test_edits: row.8,
                },
                deadline_seconds,
            },
            repository,
            agent_profile,
            preparation_id: row.10,
            retry_of: row
                .11
                .map(|value| TaskId::parse(&value).map_err(StoreError::InvalidLifecycleData))
                .transpose()?,
            provided_input: row.12,
            created_at: row.20,
            updated_at: row.21,
            deadline_at: row.23,
            phase: DevelopmentPhase::parse(&row.9)?,
            requested_input: row.13,
            candidate_commit: row.14,
            validation_manifest: row.15,
            policy_or_test_edits: row.16,
            scope_violations,
            draft_pr_url: row.18,
            failure: row.19,
            lifecycle_complete: row.24,
            lifecycle,
            model_usage,
            resource_version: u64::try_from(row.25)
                .map_err(|_| StoreError::CorruptState("invalid persisted resource version"))?,
        })
    }

    #[tracing::instrument(skip_all, fields(retry_source = %source_id))]
    #[allow(clippy::too_many_lines)] // Validation, replay compatibility, and atomic creation stay together.
    pub fn retry_development(
        &mut self,
        source_id: TaskId,
        key: &IdempotencyKey,
        provided_input: Option<&str>,
    ) -> Result<DevelopmentTask, StoreError> {
        let source = self.development_task(source_id)?;
        let action_kind = format!("development-retry:{source_id}");
        if let Some((target, kind)) = self
            .connection
            .query_row(
                "SELECT task_id, action_kind FROM development_actions WHERE action_key=?1",
                [key.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?
        {
            if kind != action_kind {
                return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
            }
            let task = self.development_task(
                TaskId::parse(&target).map_err(StoreError::InvalidLifecycleData)?,
            )?;
            let provided_input = provided_input.map(str::trim);
            if provided_input.is_some_and(str::is_empty)
                || provided_input.is_some_and(|value| value.len() > 8_192)
            {
                return Err(StoreError::InvalidInput(
                    "provided input must be 1-8192 characters",
                ));
            }
            let current_shape = task.input.request == source.input.request
                && task.provided_input.as_deref() == provided_input;
            let legacy_shape = provided_input.is_some_and(|value| {
                task.provided_input.is_none()
                    && task.input.request
                        == format!("{}\n\nMaintainer input:\n{value}", source.input.request)
            });
            if !current_shape && !legacy_shape {
                return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
            }
            return Ok(task);
        }
        let request = retry_request(&source, provided_input)?;
        let now = super::orchestration::unix_seconds()?;
        if source.deadline_at.is_some_and(|deadline| deadline <= now) {
            return Err(StoreError::TaskDeadlineExpired);
        }
        let preparation_id = self.ready_preparation(
            &source.input.repository_id,
            &source.input.start_revision,
            &source.repository.lock_sha256,
        )?;
        let id = TaskId::generate();
        let repository_snapshot = serde_json::to_string(&source.repository)
            .map_err(|_| StoreError::InvalidInput("could not serialize repository"))?;
        let profile_snapshot = serde_json::to_string(&source.agent_profile)
            .map_err(|_| StoreError::InvalidInput("could not serialize profile"))?;
        let criteria = serde_json::to_string(&source.input.acceptance_criteria)
            .map_err(|_| StoreError::InvalidInput("could not serialize acceptance criteria"))?;
        let protected_paths =
            serde_json::to_string(&source.input.change_constraints.protected_paths)
                .map_err(|_| StoreError::InvalidInput("could not serialize protected paths"))?;
        let deadline_seconds = persisted_deadline_seconds(source.input.deadline_seconds)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO development_tasks(id, repository_id, profile_id, repository_snapshot,
             profile_snapshot, start_revision, request, acceptance_criteria, protected_paths,
             allow_test_edits, phase, preparation_id, retry_of, provided_input, created_at, updated_at,
             deadline_seconds, deadline_at, lifecycle_complete)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'queued',?11,?12,?13,?14,?14,?15,?16,1)",
                params![
                    id.to_string(),
                    source.input.repository_id,
                    source.input.profile_id,
                    repository_snapshot,
                    profile_snapshot,
                    source.input.start_revision,
                    request,
                    criteria,
                    protected_paths,
                    source.input.change_constraints.allow_test_edits,
                    preparation_id,
                    source_id.to_string(),
                    provided_input.map(str::trim),
                    now,
                    deadline_seconds,
                    source.deadline_at
                ],
            )
            .map_err(StoreError::Database)?;
        insert_model_usage_collection(&transaction, id, false)?;
        super::development_lifecycle::insert_phase_events(
            &transaction,
            id,
            DevelopmentPhase::Queued,
            now,
            None,
        )?;
        transaction
            .execute(
                "INSERT INTO development_actions(action_key, task_id, action_kind, created_at)
             VALUES (?1,?2,?3,?4)",
                params![key.as_str(), id.to_string(), action_kind, now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        super::development_lifecycle::log_phase_events(id, DevelopmentPhase::Queued, None);
        tracing::info!(
            task_id = %id,
            retry_source = %source_id,
            "development retry linked"
        );
        self.development_task(id)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, transition = "input_requested"))]
    pub(crate) fn record_requested_input(
        &mut self,
        id: TaskId,
        question: &str,
    ) -> Result<bool, StoreError> {
        if question.trim().is_empty() {
            return Err(StoreError::InvalidInput("requested input is empty"));
        }
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        let changed = transaction
            .execute(
                "UPDATE development_tasks SET phase='waiting_for_input', requested_input=?1,
             updated_at=?2, resource_version=resource_version+1 WHERE id=?3 AND phase='coding'",
                params![crate::truncate_utf8(question, 1024), now, id.to_string()],
            )
            .map_err(StoreError::Database)?;
        if changed == 1 {
            super::development_lifecycle::insert_phase_events(
                &transaction,
                id,
                DevelopmentPhase::WaitingForInput,
                now,
                Some("maintainer_input_required"),
            )?;
        }
        transaction.commit().map_err(StoreError::Database)?;
        if changed == 1 {
            super::development_lifecycle::log_phase_events(
                id,
                DevelopmentPhase::WaitingForInput,
                Some("maintainer_input_required"),
            );
        }
        Ok(changed == 1)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, transition = "publication_started"))]
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
        let branch = format!("orchestrator/task-{}", task.id);
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
                "SELECT branch_name, candidate_commit, title, body, pr_url
                 FROM publication_actions WHERE task_id=?1",
                [id.to_string()],
                |row| {
                    Ok(PublicationAction {
                        branch_name: row.get(0)?,
                        candidate_commit: row.get(1)?,
                        title: row.get(2)?,
                        body: row.get(3)?,
                        completed_url: row.get(4)?,
                    })
                },
            )
            .map_err(StoreError::Database)
    }

    #[tracing::instrument(skip_all, fields(task_id = %id, transition = "publication_completed"))]
    pub(crate) fn complete_publication(
        &mut self,
        id: TaskId,
        candidate: &str,
        number: u64,
        url: &str,
    ) -> Result<(), StoreError> {
        let now = super::orchestration::unix_seconds()?;
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
                    now,
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
             updated_at=?2, resource_version=resource_version+1
             WHERE id=?3 AND phase='preparing_pr' AND candidate_commit=?4",
                params![url, now, id.to_string(), candidate],
            )
            .map_err(StoreError::Database)?;
        if updated != 1 {
            return Err(StoreError::InvalidInput(
                "task changed before PR completion",
            ));
        }
        super::development_lifecycle::insert_phase_events(
            &transaction,
            id,
            DevelopmentPhase::Completed,
            now,
            None,
        )?;
        transaction.commit().map_err(StoreError::Database)?;
        super::development_lifecycle::log_phase_events(id, DevelopmentPhase::Completed, None);
        Ok(())
    }
}

#[cfg(test)]
#[path = "development_tests.rs"]
mod tests;
