//! Generated Rust bindings for the canonical Ayni Orchestrator v1 API.
//!
//! Product semantics remain in domain and application layers. This crate
//! contains transport types, generated service contracts, and shared public
//! request-bound validation only.

/// The worker-owned `ayni.orchestrator.v1` Protocol Buffers package.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod v1 {
    tonic::include_proto!("ayni.orchestrator.v1");
}

/// Public transport-level validation used by server adapters.
pub mod validation;

/// The compiled v1 descriptor set used by reflection and contract tests.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("ayni_orchestrator_v1_descriptor");

#[cfg(test)]
mod tests {
    use super::v1;
    use prost::Message;
    use prost_types::FileDescriptorSet;
    use std::collections::BTreeSet;

    #[test]
    fn descriptor_exposes_worker_delivery_and_only_active_services()
    -> Result<(), prost::DecodeError> {
        let descriptor = FileDescriptorSet::decode(super::FILE_DESCRIPTOR_SET)?;
        let services = descriptor
            .file
            .iter()
            .flat_map(|f| &f.service)
            .filter_map(|s| s.name.as_deref())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            services,
            BTreeSet::from([
                "CertificateService",
                "ConnectionService",
                "DevelopmentTaskService",
                "ProjectConversationService",
                "ProjectService",
                "ProviderService",
                "RepositoryService",
                "RuntimeProfileService",
            ])
        );
        let tasks = descriptor
            .file
            .iter()
            .flat_map(|f| &f.service)
            .find(|s| s.name.as_deref() == Some("DevelopmentTaskService"));
        let Some(tasks) = tasks else {
            panic!("task service is required")
        };
        let methods = tasks
            .method
            .iter()
            .filter_map(|m| m.name.as_deref())
            .collect::<BTreeSet<_>>();
        assert!(methods.contains("RetryTaskPublication"));
        assert!(methods.contains("WatchDevelopmentTask"));
        Ok(())
    }

    #[test]
    fn unknown_quality_measurements_survive_round_trip() -> Result<(), prost::DecodeError> {
        let quality = v1::QualityResult {
            outcome: v1::QualityOutcome::Unavailable as i32,
            findings: vec!["Full check could not execute".into()],
            ..Default::default()
        };
        let decoded = v1::QualityResult::decode(quality.encode_to_vec().as_slice())?;
        assert_eq!(decoded.completed_targets, None);
        assert_eq!(decoded.evidence, None);
        assert_eq!(decoded, quality);
        Ok(())
    }
}
