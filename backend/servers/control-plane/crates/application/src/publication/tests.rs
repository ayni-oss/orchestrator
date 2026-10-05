use super::{
    CandidateTarget, PublicationError, PublicationIntent, PublishCandidate, Repository, publish,
};
use crate::tasks::RequestIdentity;
use std::io;

struct Memory {
    available: bool,
    git: bool,
    committed: Option<(RequestIdentity, PublicationIntent)>,
}

impl Repository for Memory {
    type Error = io::Error;

    fn publication_replay(
        &self,
        identity: &RequestIdentity,
    ) -> Result<Option<String>, Self::Error> {
        let Some((previous, _)) = &self.committed else {
            return Ok(None);
        };
        if identity.digest != previous.digest {
            return Err(io::Error::other("idempotency conflict"));
        }
        Ok(Some("change".into()))
    }

    fn publication_target(&self, id: &str) -> Result<CandidateTarget, Self::Error> {
        assert_eq!(id, "candidate");
        Ok(CandidateTarget {
            candidate_id: id.into(),
            candidate_version: 3,
            repository_id: "repository".into(),
            base_revision: "base".into(),
            git_head: self.git.then(|| "head".into()),
            available: self.available,
        })
    }

    fn commit_publication(
        &mut self,
        identity: RequestIdentity,
        intent: PublicationIntent,
    ) -> Result<String, Self::Error> {
        assert!(self.committed.is_none());
        self.committed = Some((identity, intent));
        Ok("change".into())
    }
}

fn memory() -> Memory {
    Memory {
        available: true,
        git: true,
        committed: None,
    }
}
fn command() -> PublishCandidate {
    PublishCandidate {
        candidate_id: "candidate".into(),
        identity: RequestIdentity {
            key: "publish".into(),
            digest: vec![1; 32],
            payload: vec![2; 8],
        },
    }
}

#[test]
fn publication_freezes_candidate_identity_and_retains_exact_request() -> io::Result<()> {
    let mut store = memory();
    assert_eq!(
        publish(&mut store, command()).map_err(io::Error::other)?,
        "change"
    );
    let (identity, intent) = store
        .committed
        .ok_or_else(|| io::Error::other("missing intent"))?;
    assert_eq!(identity.key, "publish");
    assert_eq!(identity.digest, vec![1; 32]);
    assert_eq!(identity.payload, vec![2; 8]);
    assert_eq!(intent.candidate_id, "candidate");
    assert_eq!(intent.candidate_version, 3);
    assert_eq!(intent.repository_id, "repository");
    assert_eq!(intent.base_revision, "base");
    assert_eq!(intent.head_revision, "head");
    Ok(())
}

#[test]
fn publication_replay_precedes_changed_candidate_availability() -> io::Result<()> {
    let mut store = memory();
    let first = publish(&mut store, command()).map_err(io::Error::other)?;
    store.available = false;
    store.git = false;
    assert_eq!(
        publish(&mut store, command()).map_err(io::Error::other)?,
        first
    );
    Ok(())
}

#[test]
fn unavailable_and_bundle_candidates_cannot_create_provider_intent() {
    for (available, git) in [(false, true), (true, false)] {
        let mut store = Memory {
            available,
            git,
            committed: None,
        };
        assert!(matches!(
            publish(&mut store, command()),
            Err(PublicationError::NotPublishable)
        ));
        assert!(store.committed.is_none());
    }
}

#[test]
fn conflicting_retry_preserves_the_original_publication() -> io::Result<()> {
    let mut store = memory();
    publish(&mut store, command()).map_err(io::Error::other)?;
    let mut changed = command();
    changed.identity.digest = vec![3; 32];
    assert!(matches!(
        publish(&mut store, changed),
        Err(PublicationError::Storage(_))
    ));
    assert_eq!(
        store
            .committed
            .as_ref()
            .map(|(identity, _)| &identity.digest),
        Some(&vec![1; 32])
    );
    Ok(())
}

#[test]
fn cancelled_or_active_attempts_do_not_schedule_automatic_publication() {
    use crate::tasks::cancellation::{AttemptState, TaskState};
    for (task, attempt) in [
        (TaskState::Cancelled, AttemptState::Failed),
        (TaskState::Running, AttemptState::Running),
        (TaskState::Running, AttemptState::Queued),
        (TaskState::Unsupported, AttemptState::Unsupported),
    ] {
        assert!(
            super::automatic_request(super::AutomaticTarget {
                candidate_id: "saved".into(),
                task_state: task,
                attempt_state: attempt,
                publication_base_branch: Some("main".into()),
            })
            .is_none()
        );
    }
}

#[test]
fn saved_failed_work_uses_the_frozen_publication_request() -> io::Result<()> {
    use crate::tasks::cancellation::{AttemptState, TaskState};
    let request = super::automatic_request(super::AutomaticTarget {
        candidate_id: "saved".into(),
        task_state: TaskState::Failed,
        attempt_state: AttemptState::Failed,
        publication_base_branch: Some("release".into()),
    })
    .ok_or_else(|| io::Error::other("saved work missing"))?;
    assert_eq!(request.key, "auto-publish:saved");
    assert_eq!(request.candidate_id, "saved");
    assert_eq!(request.base_branch, "release");
    assert!(
        super::automatic_request(super::AutomaticTarget {
            candidate_id: "saved".into(),
            task_state: TaskState::Failed,
            attempt_state: AttemptState::Failed,
            publication_base_branch: None,
        })
        .is_none()
    );
    Ok(())
}
