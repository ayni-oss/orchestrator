use super::*;
use std::{cell::Cell, io};
struct Store {
    replay: bool,
    state: TaskState,
    count: u32,
    deadline: Option<TimePoint>,
    reads: Cell<usize>,
    writes: usize,
}
impl Store {
    fn new() -> Self {
        Self {
            replay: false,
            state: TaskState::Failed,
            count: 1,
            deadline: None,
            reads: Cell::new(0),
            writes: 0,
        }
    }
}
impl Repository for Store {
    type Error = io::Error;
    fn replay_retry(&self, _: &RequestIdentity) -> io::Result<Option<String>> {
        Ok(self.replay.then(|| "saved".into()))
    }
    fn retry_facts(&self, _: &str) -> io::Result<RetryFacts> {
        self.reads.set(self.reads.get() + 1);
        Ok(RetryFacts {
            version: 4,
            state: self.state,
            attempt_count: self.count,
            max_attempts: 3,
            deadline: self.deadline,
        })
    }
    fn retry_time(&self) -> io::Result<TimePoint> {
        Ok(TimePoint {
            seconds: 10,
            nanos: 500,
        })
    }
    fn commit_retry(
        &mut self,
        _: &RetryTask,
        facts: &RetryFacts,
        _: TimePoint,
    ) -> io::Result<String> {
        assert_eq!(facts.version, 4);
        self.writes += 1;
        Ok("new".into())
    }
}
fn command() -> RetryTask {
    RetryTask {
        identity: RequestIdentity {
            key: "key".into(),
            digest: vec![1],
            payload: vec![2],
        },
        task_id: "task".into(),
    }
}
#[test]
fn replay_precedes_current_project_state_and_expired_deadline() -> Result<(), RetryError<io::Error>>
{
    let mut store = Store::new();
    store.replay = true;
    store.state = TaskState::Cancelled;
    store.deadline = Some(TimePoint {
        seconds: 1,
        nanos: 0,
    });
    assert_eq!(retry(&mut store, &command())?, "saved");
    assert_eq!(store.reads.get(), 0);
    assert_eq!(store.writes, 0);
    Ok(())
}
#[test]
fn active_cancelled_successful_and_exhausted_tasks_cannot_retry() {
    for state in [
        TaskState::Queued,
        TaskState::WaitingForRuntime,
        TaskState::Running,
        TaskState::Cancelled,
        TaskState::Succeeded,
        TaskState::Unsupported,
    ] {
        let mut store = Store::new();
        store.state = state;
        assert!(matches!(
            retry(&mut store, &command()),
            Err(RetryError::NotActionable)
        ));
        assert_eq!(store.writes, 0);
    }
    let mut store = Store::new();
    store.count = 3;
    assert!(matches!(
        retry(&mut store, &command()),
        Err(RetryError::NotActionable)
    ));
    assert_eq!(store.writes, 0);
}
#[test]
fn frozen_deadline_compares_nanoseconds_and_allows_only_unexpired_retry()
-> Result<(), RetryError<io::Error>> {
    let mut store = Store::new();
    store.deadline = Some(TimePoint {
        seconds: 10,
        nanos: 500,
    });
    assert!(matches!(
        retry(&mut store, &command()),
        Err(RetryError::DeadlineExpired)
    ));
    assert_eq!(store.writes, 0);
    store.deadline = Some(TimePoint {
        seconds: 10,
        nanos: 501,
    });
    assert_eq!(retry(&mut store, &command())?, "new");
    assert_eq!(store.writes, 1);
    Ok(())
}
