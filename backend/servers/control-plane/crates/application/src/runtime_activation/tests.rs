use super::*;
use crate::identity::InvalidGitRevision;
use std::cell::Cell;
use std::io;

struct Store {
    selected: String,
    source: String,
    binding: Option<String>,
    digest: Option<String>,
    ready: bool,
    mutations: Vec<Mutation>,
    revisions_read: Cell<usize>,
    reject_commit: bool,
}
impl Store {
    fn new() -> Self {
        Self {
            selected: "revision-1".into(),
            source: "a".repeat(40),
            binding: Some("binding".into()),
            digest: None,
            ready: false,
            mutations: vec![],
            revisions_read: Cell::new(0),
            reject_commit: false,
        }
    }
}
impl Repository for Store {
    type Error = io::Error;
    fn profile(&self, _: &str) -> io::Result<ProfileFacts> {
        Ok(ProfileFacts {
            version: 7,
            selected_revision_id: Some(self.selected.clone()),
            active_artifact_id: self.digest.as_ref().map(|_| "artifact-1".into()),
            ready: self.ready,
        })
    }
    fn revision(&self, _: &str) -> io::Result<RevisionFacts> {
        self.revisions_read.set(self.revisions_read.get() + 1);
        Ok(RevisionFacts {
            source_revision: self.source.clone(),
            verification_binding: self.binding.clone(),
        })
    }
    fn artifact_digest(&self, _: &str) -> io::Result<String> {
        self.digest
            .clone()
            .ok_or_else(|| io::Error::other("missing"))
    }
    fn commit(
        &mut self,
        _: &ActivateRuntime,
        facts: &ProfileFacts,
        mutation: Mutation,
    ) -> io::Result<()> {
        assert_eq!(facts.version, 7);
        if self.reject_commit {
            return Err(io::Error::other("stale"));
        }
        self.mutations.push(mutation);
        Ok(())
    }
}
fn request() -> Result<ActivateRuntime, InvalidGitRevision> {
    Ok(ActivateRuntime {
        profile_id: "profile-1".into(),
        revision_id: "revision-1".into(),
        source_revision: GitRevision::parse(&"a".repeat(40))?,
        verification_binding: "binding".into(),
        artifact: VerifiedArtifact {
            image: "registry.test/runtime:stable".into(),
            digest: "sha256:digest".into(),
            operating_system: "linux".into(),
            architecture: "amd64".into(),
            metadata: vec![],
        },
    })
}
#[test]
fn late_selected_revision_rejects_before_loading_unrelated_evidence()
-> Result<(), InvalidGitRevision> {
    let mut store = Store::new();
    store.selected = "revision-2".into();
    assert!(matches!(
        activate(&mut store, &request()?),
        Err(ActivationError::StaleObservation)
    ));
    assert_eq!(store.revisions_read.get(), 0);
    assert!(store.mutations.is_empty());
    Ok(())
}
#[test]
fn source_and_policy_binding_must_both_match() -> Result<(), InvalidGitRevision> {
    for change_source in [true, false] {
        let mut store = Store::new();
        if change_source {
            store.source = "b".repeat(40);
        } else {
            store.binding = Some("new-binding".into());
        }
        assert!(matches!(
            activate(&mut store, &request()?),
            Err(ActivationError::StaleObservation)
        ));
        assert!(store.mutations.is_empty());
    }
    Ok(())
}
#[test]
fn unsupported_profile_cannot_activate_even_matching_source() -> Result<(), InvalidGitRevision> {
    let mut store = Store::new();
    store.binding = None;
    assert!(matches!(
        activate(&mut store, &request()?),
        Err(ActivationError::UnsupportedProfile)
    ));
    assert!(store.mutations.is_empty());
    Ok(())
}
#[test]
fn replay_preserves_existing_digest_and_does_not_append_events()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::new();
    store.digest = Some("sha256:digest".into());
    store.ready = true;
    activate(&mut store, &request()?)?;
    assert!(store.mutations.is_empty());
    store.digest = Some("sha256:other".into());
    assert!(matches!(
        activate(&mut store, &request()?),
        Err(ActivationError::IdentityConflict)
    ));
    assert!(store.mutations.is_empty());
    Ok(())
}
#[test]
fn admitted_observation_records_once_or_restores_ready_without_replacing_artifact()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = Store::new();
    activate(&mut store, &request()?)?;
    assert_eq!(store.mutations, [Mutation::RecordArtifact]);
    store.digest = Some("sha256:digest".into());
    activate(&mut store, &request()?)?;
    assert_eq!(
        store.mutations,
        [Mutation::RecordArtifact, Mutation::RestoreReady]
    );
    Ok(())
}
#[test]
fn concurrent_profile_change_does_not_turn_commit_failure_into_success()
-> Result<(), InvalidGitRevision> {
    let mut store = Store::new();
    store.reject_commit = true;
    assert!(matches!(
        activate(&mut store, &request()?),
        Err(ActivationError::Storage(_))
    ));
    assert!(store.mutations.is_empty());
    Ok(())
}
