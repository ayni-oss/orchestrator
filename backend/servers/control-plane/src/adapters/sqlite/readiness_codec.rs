//! Explicit mapping between retained protobuf resources and readiness use cases.
use super::{CleanStore, CleanStoreError, v1};
use ayni_orchestrator_application::readiness::{
    ChangeState, Command, Context, Policy, Repository, Source, State, Status, Update,
};

pub(crate) fn status_from_wire(value: &v1::ReadinessStatus) -> Result<Status, CleanStoreError> {
    Ok(Status {
        policy: match value.policy.as_str() {
            "DRAFT_ONLY" => Policy::DraftOnly,
            "READY_AFTER_SUCCESSFUL_ASSESSMENT" => Policy::AfterSuccessfulAssessment,
            _ => return Err(CleanStoreError::CorruptState),
        },
        state: match value.state.as_str() {
            "blocked" => State::Blocked,
            "pending" => State::Pending,
            "completed" => State::Completed,
            "obsolete" => State::Obsolete,
            "manual" => State::Manual,
            _ => return Err(CleanStoreError::CorruptState),
        },
        source: match value.source.as_str() {
            "orchestrator" => Source::Orchestrator,
            "provider-observed" => Source::ProviderObserved,
            "maintainer" => Source::Maintainer,
            _ => return Err(CleanStoreError::CorruptState),
        },
        authorized_head: value.authorized_head.clone(),
        assessment_id: value.assessment_id.clone(),
        diagnostic: value.diagnostic.clone(),
    })
}

pub(super) fn status_to_wire(value: Status) -> v1::ReadinessStatus {
    v1::ReadinessStatus {
        policy: match value.policy {
            Policy::DraftOnly => "DRAFT_ONLY",
            Policy::AfterSuccessfulAssessment => "READY_AFTER_SUCCESSFUL_ASSESSMENT",
        }
        .into(),
        state: match value.state {
            State::Blocked => "blocked",
            State::Pending => "pending",
            State::Completed => "completed",
            State::Obsolete => "obsolete",
            State::Manual => "manual",
        }
        .into(),
        source: match value.source {
            Source::Orchestrator => "orchestrator",
            Source::ProviderObserved => "provider-observed",
            Source::Maintainer => "maintainer",
        }
        .into(),
        authorized_head: value.authorized_head,
        assessment_id: value.assessment_id,
        diagnostic: value.diagnostic,
    }
}

impl Repository for CleanStore {
    type Error = CleanStoreError;

    fn readiness_context(&self, command: &Command) -> Result<Context, Self::Error> {
        let change = self.change_request(&command.change_id)?;
        let assessment = self.assessment(&command.assessment_id)?;
        Ok(Context {
            change_id: change.id,
            change_version: change.resource_version,
            change_state: match v1::ChangeRequestState::try_from(change.state) {
                Ok(v1::ChangeRequestState::Draft) => ChangeState::Draft,
                Ok(v1::ChangeRequestState::Open) => ChangeState::Open,
                Ok(v1::ChangeRequestState::Closed) => ChangeState::Closed,
                Ok(v1::ChangeRequestState::Merged) => ChangeState::Merged,
                _ => return Err(CleanStoreError::CorruptState),
            },
            candidate_id: (!change.candidate_id.is_empty()).then_some(change.candidate_id),
            head: change.head_revision,
            previous: change
                .readiness
                .as_ref()
                .map(status_from_wire)
                .transpose()?,
            assessment_id: assessment.id,
            assessment_change_id: assessment.change_request_id,
            assessment_head: assessment.head_revision,
            assessment_succeeded: assessment.conclusion
                == v1::AssessmentConclusion::Succeeded as i32,
        })
    }

    fn commit_readiness(&mut self, update: Update) -> Result<(), Self::Error> {
        if self.change_request(&update.change_id)?.resource_version != update.change_version {
            return Err(CleanStoreError::StaleVersion);
        }
        self.save_readiness(
            &update.change_id,
            status_to_wire(update.status),
            update.authorize_transition,
        )
        .map(|_| ())
    }
}
