//! Readiness decisions and durable authorization independent of provider protocols.
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Policy {
    DraftOnly,
    AfterSuccessfulAssessment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeState {
    Draft,
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Blocked,
    Pending,
    Completed,
    Obsolete,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Orchestrator,
    ProviderObserved,
    Maintainer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub policy: Policy,
    pub state: State,
    pub source: Source,
    pub authorized_head: String,
    pub assessment_id: String,
    pub diagnostic: String,
}

pub struct Command {
    pub change_id: String,
    pub assessment_id: String,
    pub policy: Policy,
}

pub struct Context {
    pub change_id: String,
    pub change_version: u64,
    pub change_state: ChangeState,
    pub candidate_id: Option<String>,
    pub head: String,
    pub previous: Option<Status>,
    pub assessment_id: String,
    pub assessment_change_id: String,
    pub assessment_head: String,
    pub assessment_succeeded: bool,
}

pub struct Update {
    pub change_id: String,
    pub change_version: u64,
    pub status: Status,
    pub authorize_transition: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Retained,
    Recorded,
    RequiresVerification,
}

#[derive(Debug, Error)]
pub enum ReadinessError<E> {
    #[error("readiness persistence failed")]
    Storage(#[source] E),
    #[error("assessment does not belong to the change")]
    ParentMismatch,
    #[error("readiness transition is not authorized")]
    NotAuthorized,
}

/// The caller serializes decisions; commits preserve resources, actions and events atomically.
pub trait Repository {
    type Error: std::error::Error;

    /// Load the current change and exact assessment, without applying readiness policy.
    ///
    /// # Errors
    /// Returns an error for missing or invalid durable resources.
    fn readiness_context(&self, command: &Command) -> Result<Context, Self::Error>;

    /// Commit status, optional transition intent, and its event together.
    ///
    /// # Errors
    /// Rejects stale versions and failures to verify or retain evidence.
    fn commit_readiness(&mut self, update: Update) -> Result<(), Self::Error>;
}

/// Observe existing readiness or record why automatic readiness is blocked.
///
/// # Errors
/// Returns parent mismatch or persistence failures.
pub fn reconcile<R: Repository>(
    repository: &mut R,
    command: &Command,
) -> Result<Outcome, ReadinessError<R::Error>> {
    let context = repository
        .readiness_context(command)
        .map_err(ReadinessError::Storage)?;
    if context.assessment_change_id != context.change_id {
        return Err(ReadinessError::ParentMismatch);
    }
    let Some(status) = observation(&context, command.policy) else {
        return Ok(if eligible(&context, command.policy) {
            Outcome::RequiresVerification
        } else {
            Outcome::Retained
        });
    };
    repository
        .commit_readiness(Update {
            change_id: context.change_id,
            change_version: context.change_version,
            status,
            authorize_transition: false,
        })
        .map_err(ReadinessError::Storage)?;
    Ok(Outcome::Recorded)
}

/// Persist authorization before attempting an external readiness transition.
///
/// # Errors
/// Rejects changes without a current successful assessment and verified retained evidence.
pub fn prepare<R: Repository>(
    repository: &mut R,
    command: &Command,
) -> Result<(), ReadinessError<R::Error>> {
    let context = repository
        .readiness_context(command)
        .map_err(ReadinessError::Storage)?;
    let status = Status {
        policy: command.policy,
        state: State::Pending,
        source: Source::Orchestrator,
        authorized_head: context.assessment_head.clone(),
        assessment_id: context.assessment_id.clone(),
        diagnostic: "awaiting_provider_transition".into(),
    };
    if !authorized(&context, &status) {
        return Err(ReadinessError::NotAuthorized);
    }
    repository
        .commit_readiness(Update {
            change_id: context.change_id,
            change_version: context.change_version,
            status,
            authorize_transition: true,
        })
        .map_err(ReadinessError::Storage)
}

/// Confirm only the persisted authorization for the same current head and assessment.
///
/// # Errors
/// Rejects a stale or missing readiness intent and returns persistence failures.
pub fn confirm<R: Repository>(
    repository: &mut R,
    command: &Command,
) -> Result<(), ReadinessError<R::Error>> {
    let context = repository
        .readiness_context(command)
        .map_err(ReadinessError::Storage)?;
    let Some(mut status) = context.previous.clone() else {
        return Err(ReadinessError::NotAuthorized);
    };
    if !authorized(&context, &status) {
        return Err(ReadinessError::NotAuthorized);
    }
    status.state = State::Completed;
    status.diagnostic = "automatic_readiness_confirmed".into();
    repository
        .commit_readiness(Update {
            change_id: context.change_id,
            change_version: context.change_version,
            status,
            authorize_transition: false,
        })
        .map_err(ReadinessError::Storage)
}

/// Checks authorization against current durable facts at the persistence boundary.
#[must_use]
pub fn authorized(context: &Context, status: &Status) -> bool {
    eligible(context, status.policy)
        && context.assessment_change_id == context.change_id
        && status.state == State::Pending
        && status.assessment_id == context.assessment_id
        && status.authorized_head == context.assessment_head
}

fn eligible(context: &Context, policy: Policy) -> bool {
    policy == Policy::AfterSuccessfulAssessment
        && context.change_state == ChangeState::Draft
        && context.candidate_id.is_some()
        && context.assessment_head == context.head
        && context.assessment_succeeded
}

fn observation(context: &Context, policy: Policy) -> Option<Status> {
    let mut status = Status {
        policy,
        state: State::Blocked,
        source: Source::Orchestrator,
        authorized_head: String::new(),
        assessment_id: context.assessment_id.clone(),
        diagnostic: String::new(),
    };
    if context.change_state == ChangeState::Open {
        if let Some(previous) = &context.previous {
            if matches!(previous.state, State::Completed | State::Obsolete) {
                return None;
            }
            if previous.state == State::Pending && previous.authorized_head == context.head {
                // An uncertain provider mutation is observed, never repeated.
                status = previous.clone();
                status.state = State::Completed;
                status.source = Source::ProviderObserved;
                status.diagnostic = "ready_observed_after_uncertain_response".into();
                return Some(status);
            }
        }
        status.state = State::Manual;
        status.source = Source::Maintainer;
        status.diagnostic = "already_ready".into();
        return Some(status);
    }
    let reason = if policy == Policy::DraftOnly {
        "draft_only_policy"
    } else if context.change_state != ChangeState::Draft {
        "pull_not_open_draft"
    } else if context.candidate_id.is_none() {
        "human_pull_requires_manual_readiness"
    } else if context.assessment_head != context.head {
        "assessment_obsolete"
    } else if !context.assessment_succeeded {
        "assessment_not_successful"
    } else {
        return None;
    };
    // Historical imports cannot replace an existing status for the current head.
    if context.assessment_head != context.head && context.previous.is_some() {
        return None;
    }
    status.diagnostic = reason.into();
    Some(status)
}

#[cfg(test)]
mod tests;
