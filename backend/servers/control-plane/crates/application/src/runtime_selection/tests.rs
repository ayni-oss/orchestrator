use super::*;
use crate::identity::InvalidGitRevision;
use std::io;

struct Store {
    source: String,
    identity: Option<SelectionIdentity>,
    writes: usize,
    reject: bool,
}
impl Repository for Store {
    type Error = io::Error;
    fn selected(&self, _: &str) -> io::Result<SelectedRevision> {
        Ok(SelectedRevision {
            profile_version: 3,
            revision_id: "current".into(),
            source_revision: self.source.clone(),
            identity: self.identity.clone(),
        })
    }
    fn commit_selection(
        &mut self,
        _: &SelectRuntimeRevision,
        previous: &SelectedRevision,
    ) -> io::Result<String> {
        assert_eq!(previous.profile_version, 3);
        if self.reject {
            return Err(io::Error::other("stale"));
        }
        self.writes += 1;
        Ok("new-current".into())
    }
}
fn command() -> Result<SelectRuntimeRevision, InvalidGitRevision> {
    Ok(SelectRuntimeRevision {
        profile_id: "profile".into(),
        source_revision: GitRevision::parse(&"a".repeat(40))?,
        identity: SelectionIdentity {
            policy_binding: "policy".into(),
            image_repository: "registry.test/runtime".into(),
            source_digest: Some("source".into()),
        },
        policy_version: "1".into(),
    })
}
fn store(command: &SelectRuntimeRevision) -> Store {
    Store {
        source: command.source_revision.as_str().into(),
        identity: Some(command.identity.clone()),
        writes: 0,
        reject: false,
    }
}
#[test]
fn identical_observation_replays_only_current_selection() -> Result<(), Box<dyn std::error::Error>>
{
    let request = command()?;
    let mut store = store(&request);
    assert_eq!(select(&mut store, &request)?, "current");
    assert_eq!(store.writes, 0);
    Ok(())
}
#[test]
fn changed_source_or_policy_selects_a_new_revision_instead_of_historical_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let request = command()?;
    for field in 0..4 {
        let mut store = store(&request);
        if field == 0 {
            store.source = "b".repeat(40);
        }
        if let Some(identity) = &mut store.identity {
            match field {
                1 => identity.policy_binding = "old".into(),
                2 => identity.image_repository = "registry.test/old".into(),
                3 => identity.source_digest = None,
                _ => {}
            }
        }
        assert_eq!(select(&mut store, &request)?, "new-current");
        assert_eq!(store.writes, 1);
    }
    Ok(())
}
#[test]
fn unsupported_policy_and_stale_commit_cannot_report_success() -> Result<(), InvalidGitRevision> {
    let request = command()?;
    let mut store = store(&request);
    store.identity = None;
    assert!(matches!(
        select(&mut store, &request),
        Err(SelectionError::UnsupportedProfile)
    ));
    store.identity = Some(request.identity.clone());
    store.source = "b".repeat(40);
    store.reject = true;
    assert!(matches!(
        select(&mut store, &request),
        Err(SelectionError::Storage(_))
    ));
    assert_eq!(store.writes, 0);
    Ok(())
}
