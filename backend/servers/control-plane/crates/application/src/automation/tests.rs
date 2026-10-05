use super::*;
use std::{cell::Cell, io};
struct Store {
    replay: bool,
    supported: bool,
    stale: bool,
    reads: Cell<usize>,
    writes: usize,
}
impl Store {
    fn new() -> Self {
        Self {
            replay: false,
            supported: true,
            stale: false,
            reads: Cell::new(0),
            writes: 0,
        }
    }
}
impl Repository for Store {
    type Error = io::Error;
    fn replay_automation(&self, _: &RequestIdentity) -> io::Result<Option<String>> {
        Ok(self.replay.then(|| "saved".into()))
    }
    fn automation_facts(&self, _: &str) -> io::Result<Facts> {
        self.reads.set(self.reads.get() + 1);
        Ok(Facts {
            profile_version: 3,
            revision_id: "revision".into(),
            repository_id: "repository".into(),
            repository_version: 2,
            default_branch: "main".into(),
            installation_id: 1,
            template_version: self.supported.then(|| "template".into()),
        })
    }
    fn commit_automation(
        &mut self,
        _: &InstallAutomation,
        facts: &Facts,
        plan: &Plan,
    ) -> io::Result<String> {
        assert_eq!(facts.profile_version, 3);
        assert_eq!(plan.version, "template");
        if self.stale {
            return Err(io::Error::other("stale"));
        }
        self.writes += 1;
        Ok("new".into())
    }
}
struct Planner {
    reject: bool,
    calls: Cell<usize>,
}
impl Policy for Planner {
    type Error = io::Error;
    fn plan(&self, _: &str, _: &Facts) -> io::Result<Plan> {
        self.calls.set(self.calls.get() + 1);
        if self.reject {
            return Err(io::Error::other("invalid"));
        }
        Ok(Plan {
            version: "template".into(),
            environment_workflow_path: "producer".into(),
            assessment_workflow_path: "assessment".into(),
        })
    }
}
fn command() -> InstallAutomation {
    InstallAutomation {
        profile_id: "profile".into(),
        identity: RequestIdentity {
            key: "key".into(),
            digest: vec![1],
            payload: vec![2],
        },
    }
}
#[test]
fn replay_does_not_reevaluate_changed_profile_or_template()
-> Result<(), AdmissionError<io::Error, io::Error>> {
    let mut store = Store::new();
    store.replay = true;
    store.supported = false;
    let policy = Planner {
        reject: true,
        calls: Cell::new(0),
    };
    assert_eq!(install(&mut store, &policy, &command())?, "saved");
    assert_eq!(store.reads.get(), 0);
    assert_eq!(policy.calls.get(), 0);
    assert_eq!(store.writes, 0);
    Ok(())
}
#[test]
fn unsupported_profile_and_invalid_planning_never_record_intent() {
    let mut store = Store::new();
    store.supported = false;
    let policy = Planner {
        reject: true,
        calls: Cell::new(0),
    };
    assert!(matches!(
        install(&mut store, &policy, &command()),
        Err(AdmissionError::UnsupportedProfile)
    ));
    assert_eq!(policy.calls.get(), 0);
    store.supported = true;
    assert!(matches!(
        install(&mut store, &policy, &command()),
        Err(AdmissionError::Policy(_))
    ));
    assert_eq!(store.writes, 0);
}
#[test]
fn valid_plan_commits_once_and_stale_facts_do_not_report_success()
-> Result<(), AdmissionError<io::Error, io::Error>> {
    let mut store = Store::new();
    let policy = Planner {
        reject: false,
        calls: Cell::new(0),
    };
    assert_eq!(install(&mut store, &policy, &command())?, "new");
    assert_eq!(store.writes, 1);
    store.stale = true;
    assert!(matches!(
        install(&mut store, &policy, &command()),
        Err(AdmissionError::Storage(_))
    ));
    assert_eq!(store.writes, 1);
    Ok(())
}
