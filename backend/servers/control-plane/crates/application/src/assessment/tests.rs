use super::{
    Facts, ImportAssessment, ImportError, Receipt, Repository, RetainedAssessment, import,
};
use std::{cell::Cell, io};

struct Store {
    existing: Option<RetainedAssessment>,
    repository: &'static str,
    unavailable: bool,
    reads: Cell<usize>,
    commits: usize,
}
impl Repository for Store {
    type Error = io::Error;
    fn replay(&self, _: &str) -> io::Result<Option<RetainedAssessment>> {
        if self.unavailable {
            return Err(io::Error::other("unavailable"));
        }
        Ok(self.existing.as_ref().map(|value| RetainedAssessment {
            receipt: value.receipt.clone(),
            manifest_digest: value.manifest_digest.clone(),
        }))
    }
    fn facts(&self, _: &ImportAssessment) -> io::Result<Facts> {
        self.reads.set(self.reads.get() + 1);
        Ok(Facts {
            profile_repository_id: self.repository.into(),
        })
    }
    fn commit(&mut self, command: &ImportAssessment) -> io::Result<Receipt> {
        self.commits += 1;
        let receipt = Receipt {
            id: command.id.clone(),
            head_revision: command.head_revision.clone(),
        };
        self.existing = Some(RetainedAssessment {
            receipt: receipt.clone(),
            manifest_digest: command.manifest_digest.clone(),
        });
        Ok(receipt)
    }
}
fn store() -> Store {
    Store {
        existing: None,
        repository: "repository",
        unavailable: false,
        reads: Cell::new(0),
        commits: 0,
    }
}
fn command() -> ImportAssessment {
    ImportAssessment {
        id: "assessment".into(),
        manifest_digest: "digest".into(),
        repository_id: "repository".into(),
        change_id: "change".into(),
        profile_revision_id: "profile-revision".into(),
        head_revision: "historical-head".into(),
    }
}
#[test]
fn replay_preserves_history_without_rechecking_changed_profile_membership() -> io::Result<()> {
    let mut repository = store();
    let first = import(&mut repository, &command()).map_err(io::Error::other)?;
    repository.repository = "different";
    assert_eq!(
        import(&mut repository, &command()).map_err(io::Error::other)?,
        first
    );
    assert_eq!(repository.commits, 1);
    assert_eq!(repository.reads.get(), 1);
    Ok(())
}
#[test]
fn different_manifest_cannot_replace_an_immutable_assessment() -> io::Result<()> {
    let mut repository = store();
    import(&mut repository, &command()).map_err(io::Error::other)?;
    let mut changed = command();
    changed.manifest_digest = "different".into();
    assert!(matches!(
        import(&mut repository, &changed),
        Err(ImportError::IdentityConflict)
    ));
    assert_eq!(repository.commits, 1);
    Ok(())
}
#[test]
fn foreign_profile_cannot_authorize_assessment_retention() {
    let mut repository = store();
    repository.repository = "foreign";
    assert!(matches!(
        import(&mut repository, &command()),
        Err(ImportError::ParentMismatch)
    ));
    assert_eq!(repository.commits, 0);
}
#[test]
fn unavailable_replay_does_not_authorize_a_new_commit() {
    let mut repository = store();
    repository.unavailable = true;
    assert!(matches!(
        import(&mut repository, &command()),
        Err(ImportError::Storage(_))
    ));
    assert_eq!(repository.reads.get(), 0);
    assert_eq!(repository.commits, 0);
}
