//! Canonical private-v1 workflow transport contract.
//!
//! Product semantics remain in the `PostgreSQL` authority and control-plane adapter.
//! This crate contains only generated transport types and request-bound validation.

/// The canonical `ayni.orchestrator.v1` private workflow package.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod v1 {
    tonic::include_proto!("ayni.orchestrator.v1");
    include!(concat!(env!("OUT_DIR"), "/ayni.orchestrator.v1.serde.rs"));
}

pub use pbjson_types::Timestamp;

/// Transport-level validation shared by the trusted workflow adapter.
pub mod validation;

/// The canonical descriptor set used by trusted-loopback reflection and contract tests.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("ayni_orchestrator_v1_descriptor");

#[cfg(test)]
mod tests {
    use prost::Message as _;
    use prost_types::FileDescriptorSet;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn descriptor_exposes_only_the_canonical_workflow_services() -> Result<(), prost::DecodeError> {
        let descriptor = FileDescriptorSet::decode(super::FILE_DESCRIPTOR_SET)?;
        let services = descriptor
            .file
            .iter()
            .flat_map(|file| &file.service)
            .filter_map(|service| {
                Some((
                    service.name.as_deref()?,
                    service
                        .method
                        .iter()
                        .filter_map(|method| method.name.as_deref())
                        .collect::<BTreeSet<_>>(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            services,
            BTreeMap::from([
                (
                    "ProjectConversationService",
                    BTreeSet::from([
                        "ApprovePlan",
                        "GetPlanApproval",
                        "ListTasks",
                        "GetTask",
                        "PrepareTaskExecution",
                        "GetTaskExecution",
                        "CreatePlan",
                        "RevisePlan",
                        "GetPlanVersion",
                        "ListPlans",
                        "ListPlanVersions",
                        "ListProviderCalls",
                        "CancelTurn",
                        "AcknowledgeProviderUncertainty",
                        "CreateConversation",
                        "GetConversationSnapshot",
                        "ListConversationHistory",
                        "ListConversations",
                        "ListConversationSessions",
                        "ReplaceConversationSession",
                        "SendMessage",
                        "WatchConversation",
                    ]),
                ),
                (
                    "ProjectService",
                    BTreeSet::from([
                        "CreateProject",
                        "CreateWorkspace",
                        "ListWorkspaces",
                        "ListProjects",
                        "GetAgentConfiguration",
                        "UpdateAgentConfiguration",
                        "GetTurnInputs",
                        "GetTurnContextInputs"
                    ]),
                ),
            ])
        );
        Ok(())
    }
}
