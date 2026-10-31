//! Synthetic authority qualification: no provider requests or signing material.
use ayni_orchestrator_workflow_postgres::{
    StoreError, WorkflowStore,
    store::{
        connections::{ProviderSettings, SaveGithub, SaveProvider},
        repositories::{Connect, ObservedRepository},
        repository_inspections::{Inspect, InspectionResult},
        runtime_preparations::{
            Binding, Credential, Evidence, FreezeInputs, Operation, RuntimeProfile, StageReport,
            Start,
        },
        runtime_signers::SaveSigner,
    },
};
use sha2::{Digest as _, Sha256};
type Error = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), Error>;
fn key() -> String {
    uuid::Uuid::now_v7().to_string()
}
fn hash(input: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(input.as_bytes()))
}
struct Fixture {
    store: WorkflowStore,
    repository: String,
    inspection: String,
    registry: String,
    signer: String,
    source: String,
    freeze: FreezeInputs,
}
impl Fixture {
    #[allow(clippy::too_many_lines)]
    async fn new() -> Result<Self, Error> {
        let role = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE")?;
        ayni_orchestrator_workflow_postgres::migrate_for_runtime(
            &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
            &role,
        )
        .await?;
        let url = std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?;
        ayni_orchestrator_workflow_postgres::validate_for_runtime(&url, &role).await?;
        let store = WorkflowStore::new(url);
        let source = store
            .save_github_connection(SaveGithub {
                key: &key(),
                id: "",
                expected: 0,
                name: "Runtime source fixture",
                app_id: "123",
                private_key_pem: "synthetic-not-real-app-key",
            })
            .await?;
        store.finish_github_validation(&source.id, "").await?;
        let connect = key();
        let provider = u64::try_from(uuid::Uuid::new_v4().as_u128() % 1_000_000_000_000)? + 1;
        store
            .begin_repository_connect(Connect {
                key: &connect,
                connection_id: &source.connection_id,
                expected_revision: 1,
                installation_id: 123,
                provider_repository_id: provider,
            })
            .await?;
        let repo = store
            .finish_repository_connect(
                &connect,
                Ok(ObservedRepository {
                    provider_repository_id: provider,
                    full_name: "fixture/runtime".into(),
                    default_branch: "main".into(),
                    private: true,
                }),
            )
            .await?
            .repository
            .ok_or("missing repository")?;
        let registry = store
            .save_provider_connection(SaveProvider {
                key: &key(),
                id: "",
                expected: 0,
                name: "Runtime registry fixture",
                provider: "github",
                capability: "container_registry",
                settings: ProviderSettings {
                    registry_destination: "ghcr.io/fixture/runtime".into(),
                    push_username: "fixture-push".into(),
                    pull_username: "fixture-pull".into(),
                },
                credential: &serde_json::to_string(&["synthetic-push", "synthetic-pull"])?,
            })
            .await?;
        store
            .finish_provider_validation(&registry.id, "", &[])
            .await?;
        let signer = store
            .save_runtime_signer(SaveSigner {
                key: &key(),
                id: "",
                expected_revision: 0,
                display_name: "Public signer fixture",
                key_id: "fixture-key",
                public_key: &"a".repeat(64),
                secret_name: "ayni-runtime-signer-fixture",
                secret_uid: &uuid::Uuid::new_v4().to_string(),
            })
            .await?;
        let inspection_key = key();
        let commit = "a".repeat(40);
        store
            .begin_repository_inspection(Inspect {
                key: &inspection_key,
                repository_id: &repo.id,
                expected_connection_revision: 1,
                commit_sha: &commit,
            })
            .await?;
        let inspection = store
            .finish_repository_inspection(
                &inspection_key,
                Ok(InspectionResult {
                    policy_digest: hash("policy"),
                    lock_digest: hash("lock"),
                    lock_fingerprint: hash("fingerprint"),
                    ayni_version: "0.16.2".into(),
                    lock_schema: "1".into(),
                    tools: vec![],
                    trust_key_ids: vec!["fixture-key".into()],
                    findings: vec![],
                }),
            )
            .await?;
        let image = format!("ghcr.io/fixture/executor@{}", hash("executor"));
        let profile = RuntimeProfile {
            profile_id: "rootless-podman-vfs-userns-v1".into(),
            qualified: true,
            platform: "linux/arm64".into(),
            worker_image: image.clone(),
            executor_image: image.clone(),
            verifier_image: image,
            trusted_cli_sha256: hash("cli"),
            max_cpu_millis: 1000,
            max_memory_mib: 1024,
            max_storage_mib: 4096,
            max_seconds: 3600,
            max_builds: 1,
        };
        Ok(Self {
            store,
            repository: repo.id,
            inspection: inspection.id,
            registry: registry.connection_id,
            signer: signer.id,
            source: source.connection_id,
            freeze: FreezeInputs {
                expected_signer_revision: 1,
                source_tree: "b".repeat(40),
                policy: "policy".into(),
                lock: "lock".into(),
                sources: vec![],
                profile_json: serde_json::to_string(&profile)?,
            },
        })
    }
    fn input<'a>(&'a self, command: &'a str) -> Start<'a> {
        Start {
            key: command,
            repository_id: &self.repository,
            inspection_id: &self.inspection,
            registry_connection_id: &self.registry,
            signer_id: &self.signer,
            platform: "linux/arm64",
            max_seconds: 3600,
            max_builds: 1,
            allow_build: true,
            allow_replace: false,
        }
    }
    async fn start(&self) -> Result<Operation, Error> {
        Ok(self
            .store
            .start_runtime_preparation(self.input(&key()), self.freeze.clone())
            .await?)
    }
    async fn bind(&self, op: &Operation) -> Result<Binding, Error> {
        let claim = self.store.claim_runtime_stage(&op.id).await?;
        let binding = Binding {
            operation_id: op.id.clone(),
            stage: claim.attempt.stage,
            generation: claim.attempt.generation,
            pod_uid: uuid::Uuid::new_v4().to_string(),
            command_digest: claim.attempt.command_digest,
        };
        self.store.bind_runtime_stage(&binding).await?;
        Ok(binding)
    }
    async fn finish(
        &self,
        op: &Operation,
        outcome: &str,
        evidence: Evidence,
    ) -> Result<Operation, Error> {
        let binding = self.bind(op).await?;
        self.store
            .report_runtime_stage(&StageReport {
                binding: binding.clone(),
                outcome: outcome.into(),
                safe_reason: String::new(),
                evidence,
            })
            .await?;
        self.store.stop_runtime_stage(&binding).await?;
        // A lost stopped acknowledgment cannot reapply a transition.
        self.store.stop_runtime_stage(&binding).await?;
        Ok(self.store.get_runtime_preparation(&op.id).await?)
    }
    async fn discover(&self, op: &Operation) -> Result<Operation, Error> {
        self.finish(
            op,
            "completed",
            Evidence {
                source_digest: hash("source"),
                ..Evidence::default()
            },
        )
        .await
    }
    async fn cleanup(&self, op: &Operation) -> Result<Operation, Error> {
        self.finish(
            op,
            "completed",
            Evidence {
                cleanup_complete: true,
                ..Evidence::default()
            },
        )
        .await
    }
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
#[allow(clippy::too_many_lines)]
async fn starts_reports_pod_deletion_and_cleanup_are_durable_fenced_and_idempotent() -> TestResult {
    let f = Fixture::new().await?;
    let command = key();
    let original = f
        .store
        .start_runtime_preparation(f.input(&command), f.freeze.clone())
        .await?;
    assert_eq!(
        f.store.replay_runtime_start(&f.input(&command)).await?,
        Some(original.clone())
    );
    assert_eq!(
        f.store
            .start_runtime_preparation(f.input(&command), f.freeze.clone())
            .await?,
        original
    );
    assert!(matches!(
        f.store
            .replay_runtime_start(&Start {
                max_seconds: 60,
                ..f.input(&command)
            })
            .await,
        Err(StoreError::IdempotencyConflict)
    ));
    assert_eq!(original.frozen.source_tree, f.freeze.source_tree);
    assert_eq!(original.frozen.profile_json, f.freeze.profile_json);
    assert_eq!(
        original.frozen_input_digest,
        hash(&serde_json::to_string(&original.frozen)?)
    );
    let mut changed_freeze = f.freeze.clone();
    changed_freeze.profile_json = "{}".into();
    changed_freeze.source_tree = "c".repeat(40);
    assert_eq!(
        f.store
            .start_runtime_preparation(f.input(&command), changed_freeze)
            .await?,
        original
    );
    assert!(
        f.store
            .start_runtime_preparation(
                Start {
                    max_seconds: 3601,
                    ..f.input(&key())
                },
                f.freeze.clone()
            )
            .await
            .is_err()
    );
    let binding = f.bind(&original).await?;
    assert_eq!(
        f.store
            .claim_runtime_stage(&original.id)
            .await?
            .attempt
            .pod_uid,
        binding.pod_uid
    );
    let mut forged = binding.clone();
    forged.pod_uid = uuid::Uuid::new_v4().to_string();
    let mut stale_generation = binding.clone();
    stale_generation.generation += 1;
    assert!(
        f.store
            .runtime_stage_credential(&stale_generation)
            .await
            .is_err()
    );
    let mut wrong_command = binding.clone();
    wrong_command.command_digest = hash("other command");
    assert!(f.store.runtime_stage_input(&wrong_command).await.is_err());
    assert!(f.store.runtime_stage_input(&forged).await.is_err());
    assert!(f.store.stop_runtime_stage(&forged).await.is_err());
    assert!(matches!(
        f.store.runtime_stage_credential(&binding).await?,
        Credential::Source { .. }
    ));
    let report = StageReport {
        binding: binding.clone(),
        outcome: "completed".into(),
        safe_reason: String::new(),
        evidence: Evidence {
            source_digest: hash("source"),
            ..Evidence::default()
        },
    };
    let stopping = f.store.report_runtime_stage(&report).await?;
    assert_eq!(stopping.state, "stopping");
    assert_eq!(stopping.stage, "acquire_source");
    assert!(f.store.runtime_stage_input(&binding).await.is_err());
    assert_eq!(f.store.report_runtime_stage(&report).await?, stopping);
    f.store.stop_runtime_stage(&binding).await?;
    f.store.stop_runtime_stage(&binding).await?;
    let next = f.store.get_runtime_preparation(&original.id).await?;
    assert_eq!(next.stage, "discover");
    assert!(next.generation > original.generation);
    assert!(f.store.runtime_stage_input(&binding).await.is_err());
    assert_eq!(
        f.store.get_runtime_preparation_receipt(&command).await?,
        original
    );
    let image = format!("ghcr.io/fixture/runtime@{}", hash("manifest"));
    let next = f
        .finish(
            &next,
            "completed",
            Evidence {
                candidate_image: image,
                ..Evidence::default()
            },
        )
        .await?;
    let next = f.finish(&next, "completed", Evidence::default()).await?;
    // Node pull capability can be bootstrapped only by exact supervisor scope before UID binding.
    let claim = f.store.claim_runtime_stage(&next.id).await?;
    let bootstrap = Binding {
        operation_id: next.id.clone(),
        stage: claim.attempt.stage,
        generation: claim.attempt.generation,
        pod_uid: String::new(),
        command_digest: claim.attempt.command_digest,
    };
    assert!(
        matches!(f.store.runtime_supervisor_pull_credential(&bootstrap).await?,Credential::Registry{username,..} if username=="fixture-pull")
    );
    assert!(f.store.runtime_stage_credential(&bootstrap).await.is_err());
    let next = f
        .finish(
            &next,
            "completed",
            Evidence {
                content_verified: true,
                protected_content_root: hash("protected"),
                certificate_digest: hash("certificate"),
                ..Evidence::default()
            },
        )
        .await?;
    let next = f
        .finish(
            &next,
            "completed",
            Evidence {
                managed_compatible: true,
                managed_check_digest: hash("official managed run artifact"),
                ..Evidence::default()
            },
        )
        .await?;
    assert_eq!(next.state, "cleanup_pending");
    assert_ne!(next.state, "ready");
    let ready = f.cleanup(&next).await?;
    assert_eq!(ready.state, "ready");
    assert!(ready.evidence.cleanup_complete);
    let history = f.store.runtime_preparation_attempts(&ready.id).await?;
    assert!(history.iter().all(|a| a.state == "stopped"));
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn retry_preserves_build_allowance_and_requires_cleanup() -> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let op = f.discover(&op).await?;
    let op = f.finish(&op, "missing", Evidence::default()).await?;
    assert_eq!(op.stage, "build");
    let op = f.finish(&op, "failed", Evidence::default()).await?;
    assert_eq!(op.builds_started, 1);
    assert!(
        f.store
            .retry_runtime_preparation(&key(), &op.id, op.generation)
            .await
            .is_err()
    );
    let op = f.cleanup(&op).await?;
    let retry = key();
    let restarted = f
        .store
        .retry_runtime_preparation(&retry, &op.id, op.generation)
        .await?;
    assert_eq!(restarted.deadline_unix_ms, op.deadline_unix_ms);
    assert_eq!(restarted.builds_started, 1);
    assert_eq!(
        f.store
            .retry_runtime_preparation(&retry, &op.id, op.generation)
            .await?,
        restarted
    );
    let op = f.discover(&restarted).await?;
    let op = f.finish(&op, "missing", Evidence::default()).await?;
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    assert_eq!(claim.operation.stage, "cleanup");
    assert_eq!(claim.operation.builds_started, 1);
    let op = f.cleanup(&claim.operation).await?;
    assert_eq!(op.state, "blocked");
    assert_eq!(op.safe_reason, "build_allowance_exhausted");
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn cancellation_revision_rotation_and_deadline_cannot_admit_ready() -> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let binding = f.bind(&op).await?;
    let cancel = key();
    let cancelled = f
        .store
        .cancel_runtime_preparation(&cancel, &op.id, op.generation)
        .await?;
    assert_eq!(cancelled.state, "cancelling");
    assert!(f.store.runtime_stage_input(&binding).await.is_err());
    assert_eq!(
        f.store
            .cancel_runtime_preparation(&cancel, &op.id, op.generation)
            .await?,
        cancelled
    );
    f.store.stop_runtime_stage(&binding).await?;
    let op = f.store.get_runtime_preparation(&op.id).await?;
    let op = f.cleanup(&op).await?;
    assert_eq!(op.state, "cancelled");
    let op = f.start().await?;
    let binding = f.bind(&op).await?;
    f.store
        .set_connection_enabled(&key(), &f.source, 1, false)
        .await?;
    assert!(f.store.runtime_stage_input(&binding).await.is_err());
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    assert_eq!(claim.operation.state, "cancelling");
    f.store.stop_runtime_stage(&binding).await?;
    let op = f.store.get_runtime_preparation(&op.id).await?;
    let op = f.cleanup(&op).await?;
    assert_eq!(op.state, "superseded");
    // Deadline expiry uses fixture administrator only; runtime role cannot rewrite a deadline.
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let (client, driver) = tokio_postgres::connect(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        tokio_postgres::NoTls,
    )
    .await?;
    let task = tokio::spawn(driver);
    client
        .execute(
            "UPDATE workflow_runtime_preparations SET deadline_unix_ms=1 WHERE id=$1",
            &[&op.id],
        )
        .await?;
    task.abort();
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    assert_eq!(claim.operation.stage, "cleanup");
    let op = f.cleanup(&claim.operation).await?;
    assert_eq!(op.state, "failed");
    assert_eq!(op.safe_reason, "deadline_exceeded");
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn publication_intent_is_retained_before_effect_and_uncertain_recovery_gets_no_new_budget()
-> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let op = f.discover(&op).await?;
    let op = f.finish(&op, "missing", Evidence::default()).await?;
    let artifact = hash("unsigned");
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                artifact_digest: artifact.clone(),
                ..Evidence::default()
            },
        )
        .await?;
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                unsigned_manifest_digest: artifact.clone(),
                artifact_digest: artifact,
                protected_content_root: hash("protected"),
                ..Evidence::default()
            },
        )
        .await?;
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                certificate_digest: hash("certificate"),
                certification_layer_digest: hash("certificate layer"),
                artifact_digest: hash("certificate layer"),
                ..Evidence::default()
            },
        )
        .await?;
    let manifest = hash("final manifest");
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                expected_manifest_digest: manifest.clone(),
                ..Evidence::default()
            },
        )
        .await?;
    let binding = f.bind(&op).await?;
    assert!(
        f.store
            .worker_prepare_publication(&binding, &hash("other"))
            .await
            .is_err()
    );
    f.store
        .worker_prepare_publication(&binding, &manifest)
        .await?;
    let report = StageReport {
        binding: binding.clone(),
        outcome: "publication_uncertain".into(),
        safe_reason: "publication_uncertain".into(),
        evidence: Evidence::default(),
    };
    f.store.report_runtime_stage(&report).await?;
    f.store.stop_runtime_stage(&binding).await?;
    let op = f.store.get_runtime_preparation(&op.id).await?;
    let op = f.cleanup(&op).await?;
    assert_eq!(op.state, "publication_uncertain");
    assert_eq!(op.evidence.expected_manifest_digest, manifest);
    let retry = f
        .store
        .retry_runtime_preparation(&key(), &op.id, op.generation)
        .await?;
    assert_eq!(retry.evidence.expected_manifest_digest, manifest);
    assert_eq!(retry.builds_started, 1);
    assert_eq!(retry.deadline_unix_ms, op.deadline_unix_ms);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn signer_trust_validation_revision_and_immutable_runtime_records_are_fenced() -> TestResult {
    let f = Fixture::new().await?;
    let mut changed = f.freeze.clone();
    changed.expected_signer_revision = 2;
    assert!(matches!(
        f.store
            .start_runtime_preparation(f.input(&key()), changed)
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    let op = f.start().await?;
    f.store
        .set_runtime_signer_enabled(&key(), &f.signer, 1, false)
        .await?;
    assert!(matches!(
        f.store
            .start_runtime_preparation(f.input(&key()), f.freeze.clone())
            .await,
        Err(StoreError::ConfigurationRevisionConflict)
    ));
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    assert_eq!(claim.operation.stage, "cleanup");
    let op = f.cleanup(&claim.operation).await?;
    assert_eq!(op.state, "superseded");
    let (client, driver) = tokio_postgres::connect(
        &std::env::var("AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL")?,
        tokio_postgres::NoTls,
    )
    .await?;
    let task = tokio::spawn(driver);
    for sql in [
        "UPDATE workflow_runtime_preparations SET frozen='rewritten'",
        "UPDATE workflow_runtime_preparations SET deadline_unix_ms=1",
        "UPDATE workflow_runtime_preparations SET max_builds=10",
        "UPDATE workflow_runtime_preparation_attempts SET command_digest='rewritten'",
        "UPDATE workflow_runtime_preparation_receipts SET result='rewritten'",
        "DELETE FROM workflow_runtime_preparations",
    ] {
        let error = client
            .execute(sql, &[])
            .await
            .err()
            .ok_or("unexpected runtime mutation privilege")?;
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
        );
    }
    task.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn invalid_existing_image_requires_explicit_replacement_and_authorized_new_build()
-> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let op = f.discover(&op).await?;
    let image = format!("ghcr.io/fixture/runtime@{}", hash("invalid manifest"));
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                candidate_image: image.clone(),
                ..Evidence::default()
            },
        )
        .await?;
    let op = f.finish(&op, "completed", Evidence::default()).await?;
    let op = f.finish(&op, "invalid", Evidence::default()).await?;
    let op = f.cleanup(&op).await?;
    assert_eq!(op.state, "blocked");
    assert_eq!(op.safe_reason, "replacement_required");
    assert_eq!(op.builds_started, 0);
    let op = f
        .store
        .start_runtime_preparation(
            Start {
                allow_replace: true,
                ..f.input(&key())
            },
            f.freeze.clone(),
        )
        .await?;
    let op = f.discover(&op).await?;
    let op = f
        .finish(
            &op,
            "completed",
            Evidence {
                candidate_image: image,
                ..Evidence::default()
            },
        )
        .await?;
    let op = f.finish(&op, "completed", Evidence::default()).await?;
    let op = f.finish(&op, "invalid", Evidence::default()).await?;
    assert_eq!(op.stage, "build");
    assert!(op.evidence.candidate_image.is_empty());
    let op = f.finish(&op, "failed", Evidence::default()).await?;
    assert_eq!(op.builds_started, 1);
    let op = f.cleanup(&op).await?;
    assert_eq!(op.state, "failed");
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn cancelled_or_expired_claim_before_pod_creation_can_be_retired_without_uid() -> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    let binding = Binding {
        operation_id: op.id.clone(),
        stage: claim.attempt.stage,
        generation: claim.attempt.generation,
        pod_uid: String::new(),
        command_digest: claim.attempt.command_digest,
    };
    f.store
        .cancel_runtime_preparation(&key(), &op.id, op.generation)
        .await?;
    f.store.stop_runtime_stage(&binding).await?;
    f.store.stop_runtime_stage(&binding).await?;
    let cleanup = f.store.get_runtime_preparation(&op.id).await?;
    assert_eq!(cleanup.stage, "cleanup");
    assert_eq!(f.cleanup(&cleanup).await?.state, "cancelled");
    let op = f.start().await?;
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    let binding = Binding {
        operation_id: op.id.clone(),
        stage: claim.attempt.stage,
        generation: claim.attempt.generation,
        pod_uid: String::new(),
        command_digest: claim.attempt.command_digest,
    };
    let (client, driver) = tokio_postgres::connect(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        tokio_postgres::NoTls,
    )
    .await?;
    let task = tokio::spawn(driver);
    client
        .execute(
            "UPDATE workflow_runtime_preparations SET deadline_unix_ms=1 WHERE id=$1",
            &[&op.id],
        )
        .await?;
    task.abort();
    assert_eq!(
        f.store.claim_runtime_stage(&op.id).await?.operation.state,
        "cancelling"
    );
    f.store.stop_runtime_stage(&binding).await?;
    let cleanup = f.store.get_runtime_preparation(&op.id).await?;
    assert_eq!(f.cleanup(&cleanup).await?.state, "failed");
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn cleanup_can_resume_beyond_its_initial_lease_without_extending_effect_authority()
-> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    f.store
        .cancel_runtime_preparation(&key(), &op.id, op.generation)
        .await?;
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    assert_eq!(claim.operation.stage, "cleanup");
    let binding = Binding {
        operation_id: op.id.clone(),
        stage: claim.attempt.stage,
        generation: claim.attempt.generation,
        pod_uid: uuid::Uuid::new_v4().to_string(),
        command_digest: claim.attempt.command_digest,
    };
    let (client, driver) = tokio_postgres::connect(
        &std::env::var("AYNI_POSTGRES_FEASIBILITY_URL")?,
        tokio_postgres::NoTls,
    )
    .await?;
    let task = tokio::spawn(driver);
    client.execute("UPDATE workflow_runtime_preparation_attempts SET deadline_unix_ms=1 WHERE operation_id=$1 AND generation=$2",&[&op.id,&i64::try_from(binding.generation)?]).await?;
    task.abort();
    f.store.bind_runtime_stage(&binding).await?;
    assert!(f.store.runtime_stage_input(&binding).await.is_ok());
    assert!(matches!(
        f.store.runtime_stage_credential(&binding).await?,
        Credential::None
    ));
    f.store
        .report_runtime_stage(&StageReport {
            binding: binding.clone(),
            outcome: "completed".into(),
            safe_reason: String::new(),
            evidence: Evidence {
                cleanup_complete: true,
                ..Evidence::default()
            },
        })
        .await?;
    f.store.stop_runtime_stage(&binding).await?;
    let result = f.store.get_runtime_preparation(&op.id).await?;
    assert_eq!(result.state, "cancelled");
    assert_eq!(result.deadline_unix_ms, op.deadline_unix_ms);
    assert_eq!(result.builds_started, op.builds_started);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn known_unbound_setup_failures_retain_receipts_cleanup_and_reject_live_workloads()
-> TestResult {
    let f = Fixture::new().await?;
    let op = f.start().await?;
    let claim = f.store.claim_runtime_stage(&op.id).await?;
    let binding = Binding {
        operation_id: op.id.clone(),
        stage: claim.attempt.stage,
        generation: claim.attempt.generation,
        pod_uid: String::new(),
        command_digest: claim.attempt.command_digest,
    };
    assert!(
        f.store
            .fail_unbound_runtime_stage(&binding, "credential_or_arbitrary_error")
            .await
            .is_err()
    );
    assert!(
        f.store
            .fail_unbound_runtime_stage(&binding, "certification_failed")
            .await
            .is_err()
    );
    let mut wrong = binding.clone();
    wrong.command_digest = hash("different command");
    assert!(
        f.store
            .fail_unbound_runtime_stage(&wrong, "setup_failure")
            .await
            .is_err()
    );
    let failed = f
        .store
        .fail_unbound_runtime_stage(&binding, "setup_failure")
        .await?;
    assert_eq!(failed.stage, "cleanup");
    assert_eq!(failed.state, "cleanup_pending");
    assert_eq!(failed.safe_reason, "setup_failure");
    assert_eq!(
        f.store
            .fail_unbound_runtime_stage(&binding, "setup_failure")
            .await?,
        failed
    );
    let history = f.store.runtime_preparation_attempts(&op.id).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, "stopped");
    assert!(!history[0].report_digest.is_empty());
    let cleaned = f.cleanup(&failed).await?;
    assert_eq!(cleaned.state, "failed");
    assert_eq!(cleaned.safe_reason, "setup_failure");
    assert_eq!(cleaned.builds_started, 0);
    assert_eq!(cleaned.deadline_unix_ms, op.deadline_unix_ms);
    assert_eq!(
        f.store
            .fail_unbound_runtime_stage(&binding, "setup_failure")
            .await?,
        cleaned
    );
    let op = f.start().await?;
    let live = f.bind(&op).await?;
    assert!(
        f.store
            .fail_unbound_runtime_stage(&live, "setup_failure")
            .await
            .is_err()
    );
    let mut forged = live.clone();
    forged.pod_uid.clear();
    assert!(
        f.store
            .fail_unbound_runtime_stage(&forged, "setup_failure")
            .await
            .is_err()
    );
    assert_eq!(
        f.store.get_runtime_preparation(&op.id).await?.state,
        "running"
    );
    let cancelled = f
        .store
        .cancel_runtime_preparation(&key(), &op.id, op.generation)
        .await?;
    assert_eq!(cancelled.state, "cancelling");
    f.store.stop_runtime_stage(&live).await?;
    let cleanup = f.store.get_runtime_preparation(&op.id).await?;
    f.cleanup(&cleanup).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn local_immutable_tooling_is_admitted_but_registry_artifacts_remain_destination_scoped()
-> TestResult {
    let mut f = Fixture::new().await?;
    let mut profile: RuntimeProfile = serde_json::from_str(&f.freeze.profile_json)?;
    profile.worker_image = format!("local/ayni-runtime-worker@{}", hash("worker"));
    profile.verifier_image = format!("local/ayni-runtime-probes@{}", hash("probes"));
    f.freeze.profile_json = serde_json::to_string(&profile)?;
    let op = f.start().await?;
    assert_eq!(op.frozen.profile_json, f.freeze.profile_json);
    let mut mutable = f.freeze.clone();
    profile.worker_image = "local/ayni-runtime-worker:latest".into();
    mutable.profile_json = serde_json::to_string(&profile)?;
    assert!(
        f.store
            .start_runtime_preparation(f.input(&key()), mutable)
            .await
            .is_err()
    );
    let op = f.discover(&op).await?;
    let binding = f.bind(&op).await?;
    assert!(
        f.store
            .report_runtime_stage(&StageReport {
                binding: binding.clone(),
                outcome: "completed".into(),
                safe_reason: String::new(),
                evidence: Evidence {
                    candidate_image: format!("ghcr.io/other/runtime@{}", hash("image")),
                    ..Evidence::default()
                }
            })
            .await
            .is_err()
    );
    f.store
        .cancel_runtime_preparation(&key(), &op.id, op.generation)
        .await?;
    f.store.stop_runtime_stage(&binding).await?;
    let op = f.store.get_runtime_preparation(&op.id).await?;
    assert_eq!(f.cleanup(&op).await?.state, "cancelled");
    Ok(())
}
