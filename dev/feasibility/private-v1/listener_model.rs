//! Type-level feasibility model for the M1 listener and workflow-owner boundary.

#![allow(dead_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowService {
    Project,
    ProjectConversation,
}

mod sealed {
    pub trait Sealed {}
}

pub trait WorkflowOwner: sealed::Sealed {}

#[derive(Clone, Copy, Debug)]
pub struct PostgresWorkflowOwner;

impl sealed::Sealed for PostgresWorkflowOwner {}
impl WorkflowOwner for PostgresWorkflowOwner {}

// This type exists only for the compile-fail proof. It deliberately cannot satisfy
// the sealed WorkflowOwner bound.
#[derive(Clone, Copy, Debug)]
pub struct SqliteWorkflowOwner;

#[derive(Debug)]
pub struct TrustedOperatorListener<Owner: WorkflowOwner> {
    owner: Owner,
}

impl<Owner: WorkflowOwner> TrustedOperatorListener<Owner> {
    pub const fn new(owner: Owner) -> Self {
        Self { owner }
    }

    pub const fn workflow_services(&self) -> [WorkflowService; 2] {
        [
            WorkflowService::Project,
            WorkflowService::ProjectConversation,
        ]
    }
}

#[derive(Debug, Default)]
pub struct CustomClientListener;

impl CustomClientListener {
    pub const fn new() -> Self {
        Self
    }

    pub const fn workflow_services(&self) -> [WorkflowService; 0] {
        []
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CustomClientListener, PostgresWorkflowOwner, TrustedOperatorListener, WorkflowService,
    };

    #[test]
    fn trusted_listener_has_only_the_postgres_owned_m1_services() {
        let listener = TrustedOperatorListener::new(PostgresWorkflowOwner);
        assert_eq!(
            listener.workflow_services(),
            [
                WorkflowService::Project,
                WorkflowService::ProjectConversation
            ]
        );
    }

    #[test]
    fn custom_listener_has_no_workflow_service_registration() {
        assert_eq!(CustomClientListener::new().workflow_services(), []);
    }
}
