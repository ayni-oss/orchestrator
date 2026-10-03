//! Generated Rust bindings for the canonical Ayni Orchestrator v1 API.
//!
//! Product semantics remain in domain and application layers. This crate
//! contains transport types, generated service contracts, and shared public
//! request-bound validation only.

/// The additive-only `ayni.orchestrator.v1` Protocol Buffers package.
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
    use prost_types::{FileDescriptorSet, field_descriptor_proto::Type};
    use std::collections::{BTreeMap, BTreeSet};

    fn descriptor() -> Result<FileDescriptorSet, prost::DecodeError> {
        FileDescriptorSet::decode(super::FILE_DESCRIPTOR_SET)
    }

    #[test]
    fn descriptor_contains_the_clean_v1_services() -> Result<(), prost::DecodeError> {
        let descriptor = descriptor()?;
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
                "ReviewService",
                "RuntimeProfileService",
            ])
        );
        Ok(())
    }

    #[test]
    fn every_resource_family_has_bounded_list_and_resumable_watch() -> Result<(), prost::DecodeError>
    {
        let descriptor = descriptor()?;
        let methods = descriptor
            .file
            .iter()
            .flat_map(|file| file.service.iter())
            .filter_map(|service| {
                service.name.as_deref().map(|name| {
                    (
                        name,
                        service
                            .method
                            .iter()
                            .filter_map(|method| method.name.as_deref())
                            .collect::<BTreeSet<_>>(),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();

        assert!(methods["RepositoryService"].contains("ListRepositoryConnections"));
        assert!(methods["RepositoryService"].contains("ListRepositorySnapshots"));
        assert!(methods["RepositoryService"].contains("WatchRepositories"));
        assert!(methods["RuntimeProfileService"].contains("ListRuntimeProfiles"));
        assert!(methods["RuntimeProfileService"].contains("ListRuntimeArtifacts"));
        assert!(methods["RuntimeProfileService"].contains("ListAutomationInstallations"));
        assert!(methods["RuntimeProfileService"].contains("WatchRuntimeProfiles"));
        assert!(methods["DevelopmentTaskService"].contains("ListDevelopmentAttempts"));
        assert!(methods["DevelopmentTaskService"].contains("ListCandidates"));
        assert!(methods["DevelopmentTaskService"].contains("WatchDevelopmentTask"));
        assert!(methods["ReviewService"].contains("ListChangeRequests"));
        assert!(methods["ReviewService"].contains("ListAssessments"));
        assert!(methods["ReviewService"].contains("ListEvidenceReferences"));
        assert!(methods["ReviewService"].contains("WatchReviews"));

        for watch in [
            "WatchRepositories",
            "WatchRuntimeProfiles",
            "WatchDevelopmentTask",
            "WatchReviews",
        ] {
            let method = descriptor
                .file
                .iter()
                .flat_map(|file| file.service.iter())
                .flat_map(|service| service.method.iter())
                .find(|method| method.name.as_deref() == Some(watch));
            assert!(
                method.is_some_and(|value| {
                    value.server_streaming == Some(true) && value.client_streaming != Some(true)
                }),
                "{watch} must be server streaming"
            );
        }
        Ok(())
    }

    #[test]
    fn every_mutation_is_idempotent_and_every_watch_has_a_cursor() -> Result<(), prost::DecodeError>
    {
        let descriptor = descriptor()?;
        let messages = descriptor
            .file
            .iter()
            .flat_map(|file| file.message_type.iter())
            .filter_map(|message| message.name.as_deref().map(|name| (name, message)))
            .collect::<BTreeMap<_, _>>();
        let methods = descriptor
            .file
            .iter()
            .flat_map(|file| file.service.iter())
            .flat_map(|service| service.method.iter())
            .filter_map(|method| method.name.as_deref().map(|name| (name, method)))
            .collect::<BTreeMap<_, _>>();

        for mutation in [
            "CreateRepositoryConnection",
            "CreateRuntimeProfile",
            "InstallRepositoryAutomation",
            "ReconcileRuntimeProfile",
            "CreateDevelopmentTask",
            "CancelDevelopmentTask",
            "RetryDevelopmentTask",
            "PublishCandidate",
        ] {
            let Some(request_name) = methods[mutation]
                .input_type
                .as_deref()
                .and_then(|name| name.rsplit('.').next())
            else {
                panic!("{mutation} request type is absent");
            };
            assert!(
                messages[request_name]
                    .field
                    .iter()
                    .any(|field| field.name.as_deref() == Some("idempotency_key")),
                "{mutation} must carry an idempotency key"
            );
        }

        for watch in [
            "WatchRepositories",
            "WatchRuntimeProfiles",
            "WatchDevelopmentTask",
            "WatchReviews",
        ] {
            let Some(request_name) = methods[watch]
                .input_type
                .as_deref()
                .and_then(|name| name.rsplit('.').next())
            else {
                panic!("{watch} request type is absent");
            };
            let Some(response_name) = methods[watch]
                .output_type
                .as_deref()
                .and_then(|name| name.rsplit('.').next())
            else {
                panic!("{watch} response type is absent");
            };
            assert!(messages[request_name].field.iter().any(|field| {
                field.name.as_deref() == Some("after_cursor") && field.proto3_optional == Some(true)
            }));
            assert!(
                messages[response_name]
                    .field
                    .iter()
                    .any(|field| field.name.as_deref() == Some("cursor"))
            );
        }
        Ok(())
    }

    #[test]
    fn pilot_preparation_and_validation_surface_is_absent() -> Result<(), prost::DecodeError> {
        let encoded = descriptor()?.encode_to_vec();
        let text = String::from_utf8_lossy(&encoded);
        for retired in [
            "Preparation",
            "PrepareRepository",
            "lock_sha256",
            "validation_manifest_digest",
            "GetDevelopmentTaskEvidence",
            "ProvideDevelopmentTaskInput",
        ] {
            assert!(!text.contains(retired), "retired symbol remains: {retired}");
        }
        Ok(())
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn lifecycle_enums_expose_every_clean_v1_state() -> Result<(), prost::DecodeError> {
        let descriptor = descriptor()?;
        let enums = descriptor
            .file
            .iter()
            .flat_map(|file| file.enum_type.iter())
            .filter_map(|enumeration| {
                enumeration.name.as_deref().map(|name| {
                    (
                        name,
                        enumeration
                            .value
                            .iter()
                            .filter_map(|value| value.name.as_deref())
                            .collect::<BTreeSet<_>>(),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();

        let expected = [
            (
                "RepositoryConnectionStatus",
                [
                    "REPOSITORY_CONNECTION_STATUS_UNSPECIFIED",
                    "REPOSITORY_CONNECTION_STATUS_CONNECTING",
                    "REPOSITORY_CONNECTION_STATUS_CONNECTED",
                    "REPOSITORY_CONNECTION_STATUS_DEGRADED",
                    "REPOSITORY_CONNECTION_STATUS_REVOKED",
                    "REPOSITORY_CONNECTION_STATUS_FAILED",
                ]
                .as_slice(),
            ),
            (
                "RuntimeProfileSupport",
                [
                    "RUNTIME_PROFILE_SUPPORT_UNSPECIFIED",
                    "RUNTIME_PROFILE_SUPPORT_OPERATIONAL",
                    "RUNTIME_PROFILE_SUPPORT_UNSUPPORTED",
                ]
                .as_slice(),
            ),
            (
                "RuntimeProfileState",
                [
                    "RUNTIME_PROFILE_STATE_UNSPECIFIED",
                    "RUNTIME_PROFILE_STATE_SETUP_REQUIRED",
                    "RUNTIME_PROFILE_STATE_SETUP_PR_OPEN",
                    "RUNTIME_PROFILE_STATE_AWAITING_ENVIRONMENT",
                    "RUNTIME_PROFILE_STATE_READY",
                    "RUNTIME_PROFILE_STATE_UPDATING",
                    "RUNTIME_PROFILE_STATE_DEGRADED",
                    "RUNTIME_PROFILE_STATE_FAILED",
                ]
                .as_slice(),
            ),
            (
                "RuntimeArtifactStatus",
                [
                    "RUNTIME_ARTIFACT_STATUS_UNSPECIFIED",
                    "RUNTIME_ARTIFACT_STATUS_DISCOVERED",
                    "RUNTIME_ARTIFACT_STATUS_VERIFYING",
                    "RUNTIME_ARTIFACT_STATUS_VERIFIED",
                    "RUNTIME_ARTIFACT_STATUS_REJECTED",
                    "RUNTIME_ARTIFACT_STATUS_UNAVAILABLE",
                    "RUNTIME_ARTIFACT_STATUS_FAILED",
                ]
                .as_slice(),
            ),
            (
                "AutomationInstallationState",
                [
                    "AUTOMATION_INSTALLATION_STATE_UNSPECIFIED",
                    "AUTOMATION_INSTALLATION_STATE_SETUP_REQUIRED",
                    "AUTOMATION_INSTALLATION_STATE_SETUP_PR_OPEN",
                    "AUTOMATION_INSTALLATION_STATE_INSTALLED",
                    "AUTOMATION_INSTALLATION_STATE_UPDATING",
                    "AUTOMATION_INSTALLATION_STATE_DEGRADED",
                    "AUTOMATION_INSTALLATION_STATE_FAILED",
                ]
                .as_slice(),
            ),
            (
                "DevelopmentTaskState",
                [
                    "DEVELOPMENT_TASK_STATE_UNSPECIFIED",
                    "DEVELOPMENT_TASK_STATE_QUEUED",
                    "DEVELOPMENT_TASK_STATE_WAITING_FOR_RUNTIME",
                    "DEVELOPMENT_TASK_STATE_RUNNING",
                    "DEVELOPMENT_TASK_STATE_SUCCEEDED",
                    "DEVELOPMENT_TASK_STATE_FAILED",
                    "DEVELOPMENT_TASK_STATE_CANCELLED",
                    "DEVELOPMENT_TASK_STATE_DEADLINE_EXCEEDED",
                    "DEVELOPMENT_TASK_STATE_UNSUPPORTED",
                ]
                .as_slice(),
            ),
            (
                "DevelopmentAttemptState",
                [
                    "DEVELOPMENT_ATTEMPT_STATE_UNSPECIFIED",
                    "DEVELOPMENT_ATTEMPT_STATE_QUEUED",
                    "DEVELOPMENT_ATTEMPT_STATE_RUNNING",
                    "DEVELOPMENT_ATTEMPT_STATE_WAITING_FOR_INPUT",
                    "DEVELOPMENT_ATTEMPT_STATE_SUCCEEDED",
                    "DEVELOPMENT_ATTEMPT_STATE_FAILED",
                    "DEVELOPMENT_ATTEMPT_STATE_CANCELLED",
                    "DEVELOPMENT_ATTEMPT_STATE_DEADLINE_EXCEEDED",
                    "DEVELOPMENT_ATTEMPT_STATE_UNSUPPORTED",
                ]
                .as_slice(),
            ),
            (
                "CandidateState",
                [
                    "CANDIDATE_STATE_UNSPECIFIED",
                    "CANDIDATE_STATE_AVAILABLE",
                    "CANDIDATE_STATE_REJECTED",
                ]
                .as_slice(),
            ),
            (
                "ChangeRequestState",
                [
                    "CHANGE_REQUEST_STATE_UNSPECIFIED",
                    "CHANGE_REQUEST_STATE_PUBLISHING",
                    "CHANGE_REQUEST_STATE_DRAFT",
                    "CHANGE_REQUEST_STATE_OPEN",
                    "CHANGE_REQUEST_STATE_CLOSED",
                    "CHANGE_REQUEST_STATE_MERGED",
                    "CHANGE_REQUEST_STATE_FAILED",
                ]
                .as_slice(),
            ),
            (
                "AssessmentConclusion",
                [
                    "ASSESSMENT_CONCLUSION_UNSPECIFIED",
                    "ASSESSMENT_CONCLUSION_SUCCEEDED",
                    "ASSESSMENT_CONCLUSION_QUALITY_FAILED",
                    "ASSESSMENT_CONCLUSION_INCOMPLETE",
                    "ASSESSMENT_CONCLUSION_POLICY_REJECTED",
                    "ASSESSMENT_CONCLUSION_STALE",
                    "ASSESSMENT_CONCLUSION_CANCELLED",
                    "ASSESSMENT_CONCLUSION_INFRASTRUCTURE_FAILED",
                    "ASSESSMENT_CONCLUSION_UNSUPPORTED",
                ]
                .as_slice(),
            ),
        ];

        for (name, values) in expected {
            assert_eq!(enums[name], values.iter().copied().collect());
        }
        Ok(())
    }

    #[test]
    fn profile_and_provider_boundaries_are_closed_oneofs() -> Result<(), prost::DecodeError> {
        let descriptor = descriptor()?;
        let messages = descriptor
            .file
            .iter()
            .flat_map(|file| file.message_type.iter())
            .filter_map(|message| message.name.as_deref().map(|name| (name, message)))
            .collect::<BTreeMap<_, _>>();

        let configuration = messages["RuntimeProfileConfiguration"];
        assert_eq!(configuration.oneof_decl.len(), 1);
        assert_eq!(
            configuration
                .field
                .iter()
                .filter_map(|field| field.name.as_deref())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["ayni_managed", "custom_runtime"])
        );

        for boundary in [
            "RepositoryProvider",
            "WorkflowExecution",
            "AutomationInstallation",
            "ChangeRequestProvider",
        ] {
            assert!(
                !messages[boundary].oneof_decl.is_empty(),
                "{boundary} must isolate provider-specific fields"
            );
        }
        Ok(())
    }

    #[test]
    fn immutable_identity_fields_remain_strings_not_interchangeable_numbers()
    -> Result<(), prost::DecodeError> {
        let descriptor = descriptor()?;
        let identity_fields = [
            ("RepositorySnapshot", "revision"),
            ("AyniManagedProfileRevision", "lock_fingerprint"),
            ("RuntimeArtifact", "resolved_digest"),
            ("FrozenRuntime", "resolved_digest"),
            ("GitCandidate", "commit_sha"),
            ("ChangeRequest", "head_revision"),
            ("EvidenceReference", "digest"),
        ];
        let messages = descriptor
            .file
            .iter()
            .flat_map(|file| file.message_type.iter())
            .filter_map(|message| message.name.as_deref().map(|name| (name, message)))
            .collect::<BTreeMap<_, _>>();

        for (message, field) in identity_fields {
            let field = messages[message]
                .field
                .iter()
                .find(|candidate| candidate.name.as_deref() == Some(field));
            assert_eq!(
                field.and_then(|value| value.r#type),
                Some(Type::String as i32)
            );
        }
        Ok(())
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn review_fixtures_round_trip_managed_and_custom_boundaries() -> Result<(), prost::DecodeError>
    {
        fn round_trip<T>(value: &T) -> Result<T, prost::DecodeError>
        where
            T: Message + Default + PartialEq + std::fmt::Debug,
        {
            let decoded = T::decode(value.encode_to_vec().as_slice())?;
            assert_eq!(&decoded, value);
            Ok(decoded)
        }
        fn workflow(head_sha: String) -> v1::WorkflowExecution {
            v1::WorkflowExecution {
                provider: Some(v1::workflow_execution::Provider::Github(
                    v1::GithubWorkflowExecution {
                        workflow_id: 7,
                        run_id: 8,
                        attempt: 1,
                        event_name: "pull_request".to_owned(),
                        head_sha,
                        html_url: "https://github.com/ayni-oss/orchestrator/actions/runs/8"
                            .to_owned(),
                    },
                )),
            }
        }

        let head = "c".repeat(40);
        let ayni = round_trip(&v1::RuntimeProfileRevision {
            id: "profile-revision:ayni".to_owned(),
            runtime_profile_id: "profile:ayni".to_owned(),
            repository_snapshot_id: "snapshot:1".to_owned(),
            repository_revision: "a".repeat(40),
            revision_number: 1,
            configuration: Some(v1::runtime_profile_revision::Configuration::AyniManaged(
                v1::AyniManagedProfileRevision {
                    runtime_source_digest: None,
                    lock_fingerprint: "b".repeat(64),
                    ayni_version: "0.13.0".to_owned(),
                    workflow_template_version: "v1".to_owned(),
                    oci_repository: "ghcr.io/ayni-oss/repository/ayni-environment".to_owned(),
                },
            )),
            created_at: None,
            resource_version: 1,
        })?;
        let artifact = round_trip(&v1::RuntimeArtifact {
            id: "artifact:ayni".to_owned(),
            runtime_profile_revision_id: ayni.id.clone(),
            oci_repository: "ghcr.io/ayni-oss/repository/ayni-environment".to_owned(),
            discovery_tag: Some(format!("lock-{}", "b".repeat(64))),
            resolved_digest: format!("sha256:{}", "d".repeat(64)),
            platform: Some(v1::OciPlatform {
                operating_system: "linux".to_owned(),
                architecture: "amd64".to_owned(),
                variant: None,
            }),
            origin: Some(v1::RuntimeArtifactOrigin {
                origin: Some(v1::runtime_artifact_origin::Origin::RepositoryAutomation(
                    v1::AutomationArtifactOrigin {
                        automation_installation_id: "automation:1".to_owned(),
                        workflow: Some(workflow("a".repeat(40))),
                        repository_snapshot_id: "snapshot:1".to_owned(),
                    },
                )),
            }),
            status: v1::RuntimeArtifactStatus::Verified as i32,
            verified_metadata: vec![v1::VerifiedMetadata {
                key: "manifest_digest".to_owned(),
                value: format!("sha256:{}", "e".repeat(64)),
            }],
            failure: None,
            created_at: None,
            verified_at: None,
            resource_version: 2,
        })?;
        let change_request = round_trip(&v1::ChangeRequest {
            readiness: None,
            id: "change-request:1".to_owned(),
            repository_connection_id: "repository:1".to_owned(),
            candidate_id: "candidate:1".to_owned(),
            provider: Some(v1::ChangeRequestProvider {
                provider: Some(v1::change_request_provider::Provider::Github(
                    v1::GithubChangeRequest {
                        repository_id: "123".to_owned(),
                        pull_request_number: 42,
                        html_url: "https://github.com/ayni-oss/orchestrator/pull/42".to_owned(),
                    },
                )),
            }),
            base_revision: "a".repeat(40),
            head_revision: head.clone(),
            state: v1::ChangeRequestState::Open as i32,
            failure: None,
            created_at: None,
            updated_at: None,
            resource_version: 3,
        })?;
        let evidence = round_trip(&v1::EvidenceReference {
            id: "evidence:1".to_owned(),
            assessment_id: "assessment:ayni".to_owned(),
            name: "assessment.json".to_owned(),
            digest: format!("sha256:{}", "f".repeat(64)),
            media_type: "application/json".to_owned(),
            size_bytes: 512,
            metadata: vec![],
            verified_at: None,
            resource_version: 4,
        })?;
        let managed_assessment = round_trip(&v1::Assessment {
            id: evidence.assessment_id.clone(),
            repository_connection_id: change_request.repository_connection_id.clone(),
            change_request_id: change_request.id.clone(),
            head_revision: head.clone(),
            runtime_profile_revision_id: ayni.id.clone(),
            producer: Some(v1::AssessmentProducer {
                kind: v1::AssessmentProducerKind::AyniManaged as i32,
                name: "ayni".to_owned(),
                version: "0.13.0".to_owned(),
            }),
            workflow: Some(workflow(head.clone())),
            conclusion: v1::AssessmentConclusion::Succeeded as i32,
            details: Some(v1::assessment::Details::AyniManaged(
                v1::AyniManagedAssessment {
                    lock_fingerprint: "b".repeat(64),
                    runtime_digest: artifact.resolved_digest.clone(),
                    manifest_digest: format!("sha256:{}", "e".repeat(64)),
                    completed_target_count: 3,
                    skipped_target_count: 0,
                    warning_count: 0,
                    failure_count: 0,
                },
            )),
            evidence_reference_ids: vec![evidence.id.clone()],
            created_at: None,
            resource_version: 5,
        })?;

        let custom = round_trip(&v1::RuntimeProfileRevision {
            id: "profile-revision:custom".to_owned(),
            runtime_profile_id: "profile:custom".to_owned(),
            repository_snapshot_id: "snapshot:2".to_owned(),
            repository_revision: "a".repeat(40),
            revision_number: 1,
            configuration: Some(v1::runtime_profile_revision::Configuration::CustomRuntime(
                v1::CustomRuntimeProfileRevision {
                    image_reference: format!("registry.example/runtime@sha256:{}", "d".repeat(64)),
                },
            )),
            created_at: None,
            resource_version: 1,
        })?;
        let custom_artifact = round_trip(&v1::RuntimeArtifact {
            id: "artifact:custom".to_owned(),
            runtime_profile_revision_id: custom.id.clone(),
            oci_repository: "registry.example/runtime".to_owned(),
            discovery_tag: None,
            resolved_digest: format!("sha256:{}", "9".repeat(64)),
            platform: Some(v1::OciPlatform {
                operating_system: "linux".to_owned(),
                architecture: "amd64".to_owned(),
                variant: None,
            }),
            origin: Some(v1::RuntimeArtifactOrigin {
                origin: Some(v1::runtime_artifact_origin::Origin::External(
                    v1::ExternalArtifactOrigin {
                        supplied_reference: format!(
                            "registry.example/runtime@sha256:{}",
                            "9".repeat(64)
                        ),
                        observed_at: None,
                    },
                )),
            }),
            status: v1::RuntimeArtifactStatus::Verified as i32,
            verified_metadata: vec![],
            failure: None,
            created_at: None,
            verified_at: None,
            resource_version: 2,
        })?;
        let external_assessment = round_trip(&v1::Assessment {
            id: "assessment:external".to_owned(),
            repository_connection_id: change_request.repository_connection_id.clone(),
            change_request_id: change_request.id.clone(),
            head_revision: head.clone(),
            runtime_profile_revision_id: custom.id.clone(),
            producer: Some(v1::AssessmentProducer {
                kind: v1::AssessmentProducerKind::External as i32,
                name: "repository-checks".to_owned(),
                version: "v1".to_owned(),
            }),
            workflow: Some(workflow(head)),
            conclusion: v1::AssessmentConclusion::Succeeded as i32,
            details: Some(v1::assessment::Details::External(v1::ExternalAssessment {
                system: "repository-checks".to_owned(),
            })),
            evidence_reference_ids: vec![],
            created_at: None,
            resource_version: 6,
        })?;

        assert_eq!(ayni.id, "profile-revision:ayni");
        assert_eq!(ayni.runtime_profile_id, "profile:ayni");
        assert_eq!(ayni.repository_snapshot_id, "snapshot:1");
        assert!(matches!(
            ayni.configuration.as_ref(),
            Some(v1::runtime_profile_revision::Configuration::AyniManaged(_))
        ));
        assert_eq!(artifact.id, "artifact:ayni");
        assert_eq!(
            artifact.runtime_profile_revision_id,
            "profile-revision:ayni"
        );
        let Some(v1::RuntimeArtifactOrigin {
            origin: Some(v1::runtime_artifact_origin::Origin::RepositoryAutomation(origin)),
        }) = artifact.origin.as_ref()
        else {
            panic!("managed artifact origin did not survive the round trip");
        };
        assert_eq!(origin.repository_snapshot_id, "snapshot:1");
        assert_eq!(change_request.id, "change-request:1");
        assert_eq!(change_request.repository_connection_id, "repository:1");
        assert_eq!(change_request.candidate_id, "candidate:1");
        assert_eq!(change_request.head_revision, "c".repeat(40));
        assert_eq!(managed_assessment.id, "assessment:ayni");
        assert_eq!(managed_assessment.change_request_id, "change-request:1");
        assert_eq!(managed_assessment.head_revision, "c".repeat(40));
        assert_eq!(
            managed_assessment.runtime_profile_revision_id,
            "profile-revision:ayni"
        );
        assert_eq!(
            managed_assessment.evidence_reference_ids,
            vec!["evidence:1".to_owned()]
        );
        assert!(matches!(
            managed_assessment.details.as_ref(),
            Some(v1::assessment::Details::AyniManaged(_))
        ));
        assert_eq!(evidence.id, "evidence:1");
        assert_eq!(evidence.assessment_id, "assessment:ayni");

        assert_eq!(custom.id, "profile-revision:custom");
        assert_eq!(custom.runtime_profile_id, "profile:custom");
        assert_eq!(custom.repository_snapshot_id, "snapshot:2");
        assert!(matches!(
            custom.configuration.as_ref(),
            Some(v1::runtime_profile_revision::Configuration::CustomRuntime(
                _
            ))
        ));
        assert_eq!(custom_artifact.id, "artifact:custom");
        assert_eq!(
            custom_artifact.runtime_profile_revision_id,
            "profile-revision:custom"
        );
        let Some(v1::RuntimeArtifactOrigin {
            origin: Some(v1::runtime_artifact_origin::Origin::External(origin)),
        }) = custom_artifact.origin.as_ref()
        else {
            panic!("external artifact origin did not survive the round trip");
        };
        assert_eq!(
            origin.supplied_reference,
            format!("registry.example/runtime@sha256:{}", "9".repeat(64))
        );
        assert_eq!(external_assessment.id, "assessment:external");
        assert_eq!(external_assessment.change_request_id, "change-request:1");
        assert_eq!(external_assessment.head_revision, "c".repeat(40));
        assert_eq!(
            external_assessment.runtime_profile_revision_id,
            "profile-revision:custom"
        );
        assert!(external_assessment.evidence_reference_ids.is_empty());
        assert!(matches!(
            external_assessment.details.as_ref(),
            Some(v1::assessment::Details::External(_))
        ));
        Ok(())
    }

    #[test]
    fn generated_usage_contract_preserves_optional_token_counters() {
        let usage = v1::ProviderTokenUsage {
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
