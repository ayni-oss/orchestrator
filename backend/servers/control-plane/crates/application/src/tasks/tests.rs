use super::{
    AdmissionError, AdmissionFacts, AdmissionReceipt, AdmitTask, DeadlineDuration, FrozenTask,
    ProfileKind, RequestIdentity, RuntimeBinding, TaskRepository, TimePoint, admit,
};
use std::{cell::Cell, io};

struct Repository {
    kind: ProfileKind,
    profile_source: String,
    artifact_revision: String,
    verified: bool,
    replay: Option<AdmissionReceipt>,
    reads: Cell<usize>,
    committed: Option<FrozenTask>,
}

impl Repository {
    fn ready(kind: ProfileKind) -> Self {
        Self {
            kind,
            profile_source: "source".into(),
            artifact_revision: "revision".into(),
            verified: true,
            replay: None,
            reads: Cell::new(0),
            committed: None,
        }
    }
}

impl TaskRepository for Repository {
    type Error = io::Error;

    fn replay(&self, _: &RequestIdentity) -> Result<Option<AdmissionReceipt>, Self::Error> {
        Ok(self.replay.clone())
    }

    fn admission_facts(&self, _: &AdmitTask) -> Result<AdmissionFacts, Self::Error> {
        self.reads.set(self.reads.get() + 1);
        Ok(AdmissionFacts {
            repository_id: "repository".into(),
            snapshot_id: "snapshot".into(),
            source_revision: "source".into(),
            base_branch: "main".into(),
            profile_repository_id: "repository".into(),
            profile_ready: true,
            profile_operational: true,
            profile_source_revision: self.profile_source.clone(),
            artifact_revision_id: self.artifact_revision.clone(),
            artifact_verified: self.verified,
            runtime: Some(RuntimeBinding {
                profile_id: "profile".into(),
                revision_id: "revision".into(),
                kind: self.kind,
                artifact_id: "artifact".into(),
                oci_repository: "registry/image".into(),
                digest: "sha256:frozen".into(),
                platform: None,
            }),
        })
    }

    fn now(&self) -> Result<TimePoint, Self::Error> {
        Ok(TimePoint {
            seconds: 100,
            nanos: 900_000_000,
        })
    }

    fn commit_admission(
        &mut self,
        _: RequestIdentity,
        frozen: FrozenTask,
        _: TimePoint,
    ) -> Result<AdmissionReceipt, Self::Error> {
        self.committed = Some(frozen);
        Ok(receipt())
    }
}

fn receipt() -> AdmissionReceipt {
    AdmissionReceipt {
        task_id: "task".into(),
        attempt_id: "attempt".into(),
    }
}

fn command() -> AdmitTask {
    AdmitTask {
        identity: RequestIdentity {
            key: "key".into(),
            digest: vec![1],
            payload: vec![2],
        },
        coding_turns: Some(12),
        project_id: "project".into(),
        repository_id: "repository".into(),
        repository_revision: "source".into(),
        profile_id: "profile".into(),
        agent: None,
        request: "change".into(),
        acceptance_criteria: vec![],
        constraints: None,
        deadline: Some(DeadlineDuration {
            seconds: 2,
            nanos: 200_000_000,
        }),
    }
}

#[test]
fn freezes_source_runtime_and_normalized_deadline() -> Result<(), AdmissionError<io::Error>> {
    let mut repository = Repository::ready(ProfileKind::AyniManaged);
    assert_eq!(admit(&mut repository, command())?, receipt());
    let frozen = repository.committed.ok_or(AdmissionError::Clock)?;
    assert_eq!(frozen.source_revision, "source");
    assert_eq!(frozen.runtime.digest, "sha256:frozen");
    assert_eq!(frozen.project_id, "project");
    assert_eq!(frozen.coding_turns, Some(12));
    assert_eq!(
        frozen.deadline_at,
        Some(TimePoint {
            seconds: 103,
            nanos: 100_000_000
        })
    );
    Ok(())
}

#[test]
fn managed_source_mismatch_prevents_commit() {
    let mut repository = Repository::ready(ProfileKind::AyniManaged);
    repository.profile_source = "other-source".into();
    assert!(matches!(
        admit(&mut repository, command()),
        Err(AdmissionError::ProfileNotReady)
    ));
    assert!(repository.committed.is_none());
}

#[test]
fn custom_admission_does_not_require_managed_source_identity()
-> Result<(), AdmissionError<io::Error>> {
    let mut repository = Repository::ready(ProfileKind::CustomRuntime);
    repository.profile_source = "other-source".into();
    admit(&mut repository, command())?;
    assert!(repository.committed.is_some());
    Ok(())
}

#[test]
fn every_profile_requires_a_verified_matching_artifact() {
    for kind in [ProfileKind::AyniManaged, ProfileKind::CustomRuntime] {
        let mut repository = Repository::ready(kind);
        repository.verified = false;
        assert!(matches!(
            admit(&mut repository, command()),
            Err(AdmissionError::ProfileNotReady)
        ));
        repository.verified = true;
        repository.artifact_revision = "stale-revision".into();
        assert!(matches!(
            admit(&mut repository, command()),
            Err(AdmissionError::ProfileNotReady)
        ));
        assert!(repository.committed.is_none());
    }
}

#[test]
fn replay_retains_original_admission_after_readiness_changes()
-> Result<(), AdmissionError<io::Error>> {
    let mut repository = Repository::ready(ProfileKind::AyniManaged);
    repository.verified = false;
    repository.replay = Some(receipt());
    assert_eq!(admit(&mut repository, command())?, receipt());
    assert_eq!(repository.reads.get(), 0);
    assert!(repository.committed.is_none());
    Ok(())
}

#[test]
fn invalid_duration_cannot_create_a_task() {
    let mut repository = Repository::ready(ProfileKind::AyniManaged);
    for duration in [
        DeadlineDuration {
            seconds: -1,
            nanos: 0,
        },
        DeadlineDuration {
            seconds: 0,
            nanos: 1_000_000_000,
        },
        DeadlineDuration {
            seconds: i64::MAX,
            nanos: 0,
        },
    ] {
        let mut request = command();
        request.deadline = Some(duration);
        assert!(matches!(
            admit(&mut repository, request),
            Err(AdmissionError::Clock)
        ));
        assert!(repository.committed.is_none());
    }
}
