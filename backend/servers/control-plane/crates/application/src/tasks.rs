//! Atomic task admission and immutable execution bindings.
use ayni_orchestrator_domain::ChangeConstraints;
use thiserror::Error;

pub mod cancellation;
pub mod lifecycle;

/// Canonical persistence identity. Payload bytes are interpreted only by codecs.
pub struct RequestIdentity {
    pub key: String,
    pub digest: Vec<u8>,
    pub payload: Vec<u8>,
}

pub struct AgentSelection {
    pub provider: String,
    pub agent: String,
    pub model: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileKind {
    AyniManaged,
    CustomRuntime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimePoint {
    pub seconds: i64,
    pub nanos: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeadlineDuration {
    pub seconds: i64,
    pub nanos: i32,
}

/// Immutable planning provenance, assigned only after control-plane approval.
pub struct TaskOrigin {
    pub project_id: String,
    pub conversation_id: String,
    pub plan_id: String,
    pub plan_version: u64,
    pub proposal_id: String,
    pub research_run_id: String,
}

pub struct AdmitTask {
    pub origin: Option<TaskOrigin>,
    pub identity: RequestIdentity,
    pub project_id: String,
    pub repository_id: String,
    pub repository_revision: String,
    pub profile_id: String,
    pub agent: Option<AgentSelection>,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: Option<ChangeConstraints>,
    pub deadline: Option<DeadlineDuration>,
    pub coding_turns: Option<u32>,
}

pub struct RuntimeBinding {
    pub profile_id: String,
    pub revision_id: String,
    pub kind: ProfileKind,
    pub artifact_id: String,
    pub oci_repository: String,
    pub digest: String,
    pub platform: Option<Platform>,
}

pub struct Platform {
    pub operating_system: String,
    pub architecture: String,
    pub variant: Option<String>,
}

pub struct AdmissionFacts {
    pub repository_id: String,
    pub snapshot_id: String,
    pub source_revision: String,
    pub base_branch: String,
    pub profile_repository_id: String,
    pub profile_ready: bool,
    pub profile_operational: bool,
    pub profile_source_revision: String,
    pub artifact_revision_id: String,
    pub artifact_verified: bool,
    pub runtime: Option<RuntimeBinding>,
}

pub struct FrozenTask {
    pub origin: Option<TaskOrigin>,
    pub project_id: String,
    pub repository_id: String,
    pub snapshot_id: String,
    pub source_revision: String,
    pub base_branch: String,
    pub runtime: RuntimeBinding,
    pub agent: Option<AgentSelection>,
    pub request: String,
    pub acceptance_criteria: Vec<String>,
    pub constraints: Option<ChangeConstraints>,
    pub deadline_at: Option<TimePoint>,
    pub coding_turns: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionReceipt {
    pub task_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Error)]
pub enum AdmissionError<E> {
    #[error("task admission persistence failed")]
    Storage(#[source] E),
    #[error("the runtime profile does not belong to the project repository")]
    ParentMismatch,
    #[error("the runtime profile has no ready matching verified artifact")]
    ProfileNotReady,
    #[error("task deadline is outside the supported clock range")]
    Clock,
}

/// The caller serializes admission; commit atomically stores task, attempt and events.
pub trait TaskRepository {
    type Error: std::error::Error;
    /// Load the original receipt for an identical request.
    ///
    /// # Errors
    /// Returns a persistence error or an idempotency conflict for a different request.
    fn replay(&self, identity: &RequestIdentity) -> Result<Option<AdmissionReceipt>, Self::Error>;
    /// Load project membership and the active immutable runtime bindings.
    ///
    /// # Errors
    /// Returns a persistence error for unavailable or invalid durable resources.
    fn admission_facts(&self, command: &AdmitTask) -> Result<AdmissionFacts, Self::Error>;
    /// Read the admission clock.
    ///
    /// # Errors
    /// Returns an error when the clock cannot be represented.
    fn now(&self) -> Result<TimePoint, Self::Error>;
    /// Commit the task, initial attempt, idempotency record and events together.
    ///
    /// # Errors
    /// Returns an error if the whole transaction cannot commit.
    fn commit_admission(
        &mut self,
        identity: RequestIdentity,
        frozen: FrozenTask,
        created_at: TimePoint,
    ) -> Result<AdmissionReceipt, Self::Error>;
}

/// Admit or replay one task while preserving its exact source and runtime identity.
///
/// # Errors
/// Rejects mismatched parents, unavailable artifacts, invalid deadlines and storage failures.
pub fn admit<R: TaskRepository>(
    repository: &mut R,
    command: AdmitTask,
) -> Result<AdmissionReceipt, AdmissionError<R::Error>> {
    if let Some(receipt) = repository
        .replay(&command.identity)
        .map_err(AdmissionError::Storage)?
    {
        return Ok(receipt);
    }
    let facts = repository
        .admission_facts(&command)
        .map_err(AdmissionError::Storage)?;
    if facts.profile_repository_id != facts.repository_id {
        return Err(AdmissionError::ParentMismatch);
    }
    if !facts.profile_ready || !facts.profile_operational || !facts.artifact_verified {
        return Err(AdmissionError::ProfileNotReady);
    }
    let runtime = facts.runtime.ok_or(AdmissionError::ProfileNotReady)?;
    if facts.artifact_revision_id != runtime.revision_id {
        return Err(AdmissionError::ProfileNotReady);
    }
    if runtime.kind == ProfileKind::AyniManaged
        && facts.source_revision != facts.profile_source_revision
    {
        return Err(AdmissionError::ProfileNotReady);
    }
    let now = repository.now().map_err(AdmissionError::Storage)?;
    let deadline_at = command
        .deadline
        .map(|duration| deadline(now, duration))
        .transpose()
        .map_err(|()| AdmissionError::Clock)?;
    let frozen = FrozenTask {
        origin: command.origin,
        project_id: command.project_id,
        repository_id: facts.repository_id,
        snapshot_id: facts.snapshot_id,
        source_revision: facts.source_revision,
        base_branch: facts.base_branch,
        runtime,
        agent: command.agent,
        request: command.request,
        acceptance_criteria: command.acceptance_criteria,
        constraints: command.constraints,
        deadline_at,
        coding_turns: command.coding_turns,
    };
    repository
        .commit_admission(command.identity, frozen, now)
        .map_err(AdmissionError::Storage)
}

fn deadline(now: TimePoint, duration: DeadlineDuration) -> Result<TimePoint, ()> {
    if duration.seconds < 0 || !(0..1_000_000_000).contains(&duration.nanos) {
        return Err(());
    }
    let nanos = i64::from(now.nanos) + i64::from(duration.nanos);
    let seconds = now
        .seconds
        .checked_add(duration.seconds)
        .and_then(|value| value.checked_add(nanos / 1_000_000_000))
        .ok_or(())?;
    Ok(TimePoint {
        seconds,
        nanos: i32::try_from(nanos % 1_000_000_000).map_err(|_| ())?,
    })
}

#[cfg(test)]
mod tests;

pub mod deadline;
pub mod retry;
