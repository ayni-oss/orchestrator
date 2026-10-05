use super::{
    ChangeState, Command, Context, Outcome, Policy, ReadinessError, Repository, Source, State,
    Status, Update, authorized, reconcile,
};
use std::io;

struct Memory {
    context: Context,
    updates: Vec<Update>,
}

impl Repository for Memory {
    type Error = io::Error;

    fn readiness_context(&self, command: &Command) -> Result<Context, Self::Error> {
        assert_eq!(command.change_id, self.context.change_id);
        assert_eq!(command.assessment_id, self.context.assessment_id);
        Ok(Context {
            change_id: self.context.change_id.clone(),
            change_version: self.context.change_version,
            change_state: self.context.change_state,
            candidate_id: self.context.candidate_id.clone(),
            head: self.context.head.clone(),
            previous: self.context.previous.clone(),
            assessment_id: self.context.assessment_id.clone(),
            assessment_change_id: self.context.assessment_change_id.clone(),
            assessment_head: self.context.assessment_head.clone(),
            assessment_succeeded: self.context.assessment_succeeded,
        })
    }

    fn commit_readiness(&mut self, update: Update) -> Result<(), Self::Error> {
        assert_eq!(update.change_version, self.context.change_version);
        self.context.previous = Some(update.status.clone());
        self.updates.push(update);
        Ok(())
    }
}

fn memory() -> Memory {
    Memory {
        context: Context {
            change_id: "change".into(),
            change_version: 4,
            change_state: ChangeState::Draft,
            candidate_id: Some("candidate".into()),
            head: "current".into(),
            previous: None,
            assessment_id: "assessment".into(),
            assessment_change_id: "change".into(),
            assessment_head: "current".into(),
            assessment_succeeded: true,
        },
        updates: Vec::new(),
    }
}

fn command(policy: Policy) -> Command {
    Command {
        change_id: "change".into(),
        assessment_id: "assessment".into(),
        policy,
    }
}

fn pending() -> Status {
    Status {
        policy: Policy::AfterSuccessfulAssessment,
        state: State::Pending,
        source: Source::Orchestrator,
        authorized_head: "current".into(),
        assessment_id: "assessment".into(),
        diagnostic: "awaiting_provider_transition".into(),
    }
}

#[test]
fn successful_current_assessment_requires_independent_provider_verification() -> io::Result<()> {
    let mut store = memory();
    let result = reconcile(&mut store, &command(Policy::AfterSuccessfulAssessment))
        .map_err(io::Error::other)?;
    assert_eq!(result, Outcome::RequiresVerification);
    assert!(store.updates.is_empty());
    assert!(authorized(&store.context, &pending()));
    Ok(())
}

#[test]
fn uncertain_provider_transition_is_observed_and_not_repeated() -> io::Result<()> {
    let mut store = memory();
    store.context.change_state = ChangeState::Open;
    store.context.previous = Some(pending());
    assert_eq!(
        reconcile(&mut store, &command(Policy::AfterSuccessfulAssessment))
            .map_err(io::Error::other)?,
        Outcome::Recorded
    );
    assert_eq!(store.updates.len(), 1);
    assert_eq!(store.updates[0].status.state, State::Completed);
    assert_eq!(store.updates[0].status.source, Source::ProviderObserved);
    assert_eq!(
        reconcile(&mut store, &command(Policy::AfterSuccessfulAssessment))
            .map_err(io::Error::other)?,
        Outcome::Retained
    );
    assert_eq!(store.updates.len(), 1);
    Ok(())
}

#[test]
fn historical_assessment_preserves_current_transition_intent() -> io::Result<()> {
    let mut store = memory();
    store.context.previous = Some(pending());
    store.context.assessment_head = "old".into();
    assert_eq!(
        reconcile(&mut store, &command(Policy::AfterSuccessfulAssessment))
            .map_err(io::Error::other)?,
        Outcome::Retained
    );
    assert!(store.updates.is_empty());
    assert!(!authorized(&store.context, &pending()));
    Ok(())
}

#[test]
fn automatic_readiness_rejects_human_changes_failed_evidence_and_draft_only_policy()
-> io::Result<()> {
    for case in 0..3 {
        let mut store = memory();
        let policy = match case {
            0 => {
                store.context.candidate_id = None;
                Policy::AfterSuccessfulAssessment
            }
            1 => {
                store.context.assessment_succeeded = false;
                Policy::AfterSuccessfulAssessment
            }
            _ => Policy::DraftOnly,
        };
        assert_eq!(
            reconcile(&mut store, &command(policy)).map_err(io::Error::other)?,
            Outcome::Recorded
        );
        assert_eq!(store.updates[0].status.state, State::Blocked);
        assert!(!authorized(&store.context, &store.updates[0].status));
    }
    Ok(())
}

#[test]
fn assessment_from_another_change_cannot_update_readiness() {
    let mut store = memory();
    store.context.assessment_change_id = "another".into();
    assert!(matches!(
        reconcile(&mut store, &command(Policy::AfterSuccessfulAssessment)),
        Err(ReadinessError::ParentMismatch)
    ));
    assert!(store.updates.is_empty());
    assert!(!authorized(&store.context, &pending()));
}

#[test]
fn transition_authority_binds_assessment_head_and_status() {
    let store = memory();
    let mut status = pending();
    status.assessment_id = "another".into();
    assert!(!authorized(&store.context, &status));
    status = pending();
    status.authorized_head = "old".into();
    assert!(!authorized(&store.context, &status));
    status = pending();
    status.state = State::Completed;
    assert!(!authorized(&store.context, &status));
}

#[test]
fn readiness_authorization_is_persisted_before_confirmation() -> io::Result<()> {
    let mut store = memory();
    let command = command(Policy::AfterSuccessfulAssessment);
    assert!(matches!(
        super::confirm(&mut store, &command),
        Err(ReadinessError::NotAuthorized)
    ));
    super::prepare(&mut store, &command).map_err(io::Error::other)?;
    assert_eq!(store.updates.len(), 1);
    assert!(store.updates[0].authorize_transition);
    assert_eq!(store.updates[0].status.state, State::Pending);
    super::confirm(&mut store, &command).map_err(io::Error::other)?;
    assert_eq!(store.updates.len(), 2);
    assert!(!store.updates[1].authorize_transition);
    assert_eq!(store.updates[1].status.state, State::Completed);
    Ok(())
}

#[test]
fn changed_head_cannot_confirm_an_earlier_provider_transition() -> io::Result<()> {
    let mut store = memory();
    let command = command(Policy::AfterSuccessfulAssessment);
    super::prepare(&mut store, &command).map_err(io::Error::other)?;
    store.context.head = "new".into();
    assert!(matches!(
        super::confirm(&mut store, &command),
        Err(ReadinessError::NotAuthorized)
    ));
    assert_eq!(store.updates.len(), 1);
    Ok(())
}

#[test]
fn existing_ready_change_without_intent_is_attributed_to_maintainer() -> io::Result<()> {
    let mut store = memory();
    store.context.change_state = ChangeState::Open;
    reconcile(&mut store, &command(Policy::AfterSuccessfulAssessment)).map_err(io::Error::other)?;
    assert_eq!(store.updates[0].status.state, State::Manual);
    assert_eq!(store.updates[0].status.source, Source::Maintainer);
    assert!(!store.updates[0].authorize_transition);
    Ok(())
}
