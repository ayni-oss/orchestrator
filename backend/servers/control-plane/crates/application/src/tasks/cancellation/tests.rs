use super::{
    AttemptState, AttemptTarget, CancelTask, CancellationError, CancellationPlan,
    CancellationRepository, CancellationTarget, TaskState, cancel,
};
use crate::tasks::RequestIdentity;
use std::{cell::Cell, io};

struct Repository {
    task: TaskState,
    attempt: AttemptState,
    replay: bool,
    reads: Cell<usize>,
    plan: Option<CancellationPlan>,
}

impl Repository {
    fn new(task: TaskState, attempt: AttemptState) -> Self {
        Self {
            task,
            attempt,
            replay: false,
            reads: Cell::new(0),
            plan: None,
        }
    }
}

impl CancellationRepository for Repository {
    type Error = io::Error;

    fn cancellation_replay(&self, _: &RequestIdentity) -> Result<Option<String>, Self::Error> {
        Ok(self.replay.then(|| "task".into()))
    }

    fn cancellation_target(&self, _: &str) -> Result<CancellationTarget, Self::Error> {
        self.reads.set(self.reads.get() + 1);
        Ok(CancellationTarget {
            task_id: "task".into(),
            state: self.task,
            version: 5,
            attempt: Some(AttemptTarget {
                id: "attempt".into(),
                state: self.attempt,
                version: 3,
            }),
        })
    }

    fn commit_cancellation(
        &mut self,
        _: RequestIdentity,
        plan: CancellationPlan,
    ) -> Result<String, Self::Error> {
        self.plan = Some(plan);
        Ok("task".into())
    }
}

fn command() -> CancelTask {
    CancelTask {
        identity: RequestIdentity {
            key: "key".into(),
            digest: vec![1],
            payload: vec![2],
        },
        task_id: "task".into(),
    }
}

#[test]
fn active_tasks_cancel_only_unfinished_attempts() -> Result<(), CancellationError<io::Error>> {
    for task in [
        TaskState::Queued,
        TaskState::WaitingForRuntime,
        TaskState::Running,
    ] {
        for attempt in [
            AttemptState::Queued,
            AttemptState::Running,
            AttemptState::WaitingForInput,
        ] {
            let mut repository = Repository::new(task, attempt);
            cancel(&mut repository, command())?;
            assert!(
                repository
                    .plan
                    .as_ref()
                    .is_some_and(|plan| plan.cancel_attempt)
            );
        }
        for attempt in [
            AttemptState::Succeeded,
            AttemptState::Failed,
            AttemptState::Cancelled,
            AttemptState::DeadlineExceeded,
            AttemptState::Unsupported,
        ] {
            let mut repository = Repository::new(task, attempt);
            cancel(&mut repository, command())?;
            assert!(
                repository
                    .plan
                    .as_ref()
                    .is_some_and(|plan| !plan.cancel_attempt)
            );
        }
    }
    Ok(())
}

#[test]
fn completed_tasks_reject_new_cancellation_without_committing() {
    for state in [
        TaskState::Succeeded,
        TaskState::Failed,
        TaskState::DeadlineExceeded,
        TaskState::Unsupported,
    ] {
        let mut repository = Repository::new(state, AttemptState::Succeeded);
        assert!(matches!(
            cancel(&mut repository, command()),
            Err(CancellationError::NotActionable)
        ));
        assert!(repository.plan.is_none());
    }
}

#[test]
fn already_cancelled_tasks_record_identity_without_recancelling_attempts()
-> Result<(), CancellationError<io::Error>> {
    let mut repository = Repository::new(TaskState::Cancelled, AttemptState::Cancelled);
    cancel(&mut repository, command())?;
    assert!(
        repository
            .plan
            .as_ref()
            .is_some_and(|plan| !plan.cancel_attempt && plan.target.state == TaskState::Cancelled)
    );
    Ok(())
}

#[test]
fn durable_replay_precedes_current_task_policy() -> Result<(), CancellationError<io::Error>> {
    let mut repository = Repository::new(TaskState::Succeeded, AttemptState::Succeeded);
    repository.replay = true;
    assert_eq!(cancel(&mut repository, command())?, "task");
    assert_eq!(repository.reads.get(), 0);
    assert!(repository.plan.is_none());
    Ok(())
}
