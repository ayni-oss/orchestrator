//! Generated Rust bindings for the canonical Ayni Orchestrator v1 API.
//!
//! Product semantics remain in the domain and application layers. This crate
//! contains transport types and generated service contracts only.

/// The additive-only `ayni.orchestrator.v1` Protocol Buffers package.
#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod v1 {
    tonic::include_proto!("ayni.orchestrator.v1");
}

/// The compiled v1 descriptor set used by reflection and contract tests.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("ayni_orchestrator_v1_descriptor");

#[cfg(test)]
mod tests {
    use prost::Message;
    use prost_types::FileDescriptorSet;
    use std::collections::BTreeSet;

    #[test]
    fn descriptor_contains_the_public_v1_services() -> Result<(), prost::DecodeError> {
        let descriptor = FileDescriptorSet::decode(super::FILE_DESCRIPTOR_SET)?;
        let services = descriptor
            .file
            .iter()
            .filter(|file| file.package.as_deref() == Some("ayni.orchestrator.v1"))
            .flat_map(|file| file.service.iter())
            .filter_map(|service| service.name.as_deref())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            services,
            BTreeSet::from([
                "DevelopmentTaskService",
                "ProviderService",
                "RepositoryService",
            ])
        );

        let development = descriptor
            .file
            .iter()
            .flat_map(|file| file.service.iter())
            .find(|service| service.name.as_deref() == Some("DevelopmentTaskService"));
        let Some(development) = development else {
            panic!("development task service is absent");
        };
        let methods = development
            .method
            .iter()
            .filter_map(|method| method.name.as_deref())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            methods,
            BTreeSet::from([
                "CancelDevelopmentTask",
                "CreateDevelopmentTask",
                "GetDevelopmentTask",
                "GetDevelopmentTaskEvidence",
                "ListDevelopmentTasks",
                "ProvideDevelopmentTaskInput",
                "RetryDevelopmentTask",
                "WatchDevelopmentTask",
            ])
        );
        assert!(
            development.method.iter().any(|method| {
                method.name.as_deref() == Some("WatchDevelopmentTask")
                    && method.server_streaming == Some(true)
                    && method.client_streaming != Some(true)
            }),
            "task watch must remain server streaming"
        );
        Ok(())
    }

    #[test]
    fn generated_task_contract_preserves_optional_token_usage() {
        let usage = super::v1::ProviderTokenUsage {
            input_tokens: None,
            output_tokens: Some(42),
            cached_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(42),
        };

        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, Some(42));
    }
}
