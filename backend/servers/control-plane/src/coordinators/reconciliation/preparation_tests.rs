//! Durable image phase tests; provider and Kubernetes observations are controlled fixtures.
use super::*;
use ayni_orchestrator_application::execution::{ExecutionResult, ExecutionState, WorkloadRuntime};
use ayni_orchestrator_runtime::{
    coding::WorkerSpec, runtime_execution::ExecutionRequest as GenericRequest,
};
use std::{collections::BTreeSet, sync::Mutex as StdMutex};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
#[derive(Default)]
struct Jobs {
    states: BTreeMap<String, ExecutionState>,
    builds: BTreeSet<String>,
    publications: BTreeMap<String, String>,
    discovery: Option<ImageDiscovery>,
    verification_failure: Option<&'static str>,
    verification_reads: usize,
    candidate_missing: bool,
    cleanup_failed: bool,
    cleanup_pending: bool,
    lost_engines: BTreeSet<String>,
}
#[derive(Clone, Default)]
struct Backend(Arc<StdMutex<Jobs>>);
impl ExecutionBackend for Backend {
    async fn start_trust_lock<P: Serialize + Sync>(
        &self,
        _: &str,
        _: &WorkloadRuntime,
        _: &GenericRequest<P>,
        _: &str,
        _: &str,
        _: &str,
    ) -> ExecutionResult<()> {
        Err("unexpected_trust_preparation".into())
    }
    async fn trust_lock_result(&self, _: &str) -> ExecutionResult<String> {
        Err("unexpected_trust_preparation".into())
    }
    async fn engine_alive(&self, name: &str, _: &WorkloadRuntime) -> ExecutionResult<bool> {
        Ok(!self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lost_engines
            .contains(name))
    }
    async fn start_discovery(&self, _: &str, _: &WorkloadRuntime) -> ExecutionResult<()> {
        Ok(())
    }
    async fn discover(&self, _: &str) -> ExecutionResult<ImageDiscovery> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .discovery
            .clone()
            .unwrap_or(ImageDiscovery::Pending))
    }
    async fn start<P: Serialize + Sync>(
        &self,
        _: &str,
        _: &WorkloadRuntime,
        _: &GenericRequest<P>,
        _: &str,
        _: Option<&ayni_orchestrator_runtime::verification::ReportingAdmission>,
    ) -> ExecutionResult<()> {
        Err("unexpected_verification".into())
    }
    async fn start_image_build<P: Serialize + Sync>(
        &self,
        name: &str,
        _: &WorkloadRuntime,
        request: &GenericRequest<P>,
        _: &str,
        kind: &ayni_orchestrator_application::image_build::ImageBuildKind,
    ) -> ExecutionResult<()> {
        assert!(matches!(
            kind,
            ayni_orchestrator_application::image_build::ImageBuildKind::RepositoryRuntime
        ));
        assert_eq!(request.revision, "a".repeat(40));
        let mut jobs = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.builds.insert(name.into());
        jobs.states.insert(name.into(), ExecutionState::Running);
        drop(jobs);
        Ok(())
    }
    async fn build_result(&self, _: &str) -> ExecutionResult<String> {
        Ok(format!("sha256:{}", "b".repeat(64)))
    }
    async fn start_publish<P: Serialize + Sync>(
        &self,
        name: &str,
        _: &WorkloadRuntime,
        _: &GenericRequest<P>,
        image_id: &str,
        _: bool,
    ) -> ExecutionResult<()> {
        let mut jobs = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.publications.insert(name.into(), image_id.into());
        jobs.states.insert(name.into(), ExecutionState::Running);
        drop(jobs);
        Ok(())
    }
    async fn operation_failure(&self, _: &str, _: &str) -> ExecutionResult<String> {
        Ok(if self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .candidate_missing
        {
            "runtime_candidate_missing"
        } else {
            "runtime_publication_failed"
        }
        .into())
    }
    async fn start_coding<P: Serialize + Sync>(
        &self,
        _: &str,
        _: &WorkloadRuntime,
        _: &WorkerSpec<P>,
        _: &str,
        _: &str,
        _: u32,
    ) -> ExecutionResult<()> {
        Err("unexpected_coding".into())
    }
    async fn refresh_credentials(&self, _: &str, _: &str) -> ExecutionResult<()> {
        Err("unexpected_credentials".into())
    }
    async fn observe(&self, _: &str) -> ExecutionResult<Observation> {
        let mut jobs = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.verification_reads += 1;
        Ok(jobs
            .verification_failure
            .map_or(Observation::Missing, Observation::Failed))
    }
    async fn coding_state(&self, name: &str) -> ExecutionResult<ExecutionState> {
        Ok(*self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .get(name)
            .unwrap_or(&ExecutionState::Missing))
    }
    async fn cleanup(&self, name: &str) -> ExecutionResult<()> {
        if self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cleanup_pending
        {
            return Err("runtime_cleanup_pending".into());
        }
        if self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cleanup_failed
        {
            return Err("cleanup_failed".into());
        }
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .remove(name);
        Ok(())
    }
    async fn coding_stopped(&self, _: &str) -> ExecutionResult<bool> {
        Ok(true)
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    reconciler: RuntimeReconciler<Backend>,
    profile: String,
    runtime: ProfileRuntime,
    cursor: RuntimeCursor,
}
async fn create_profile(state: &AppState) -> TestResult<(String, String)> {
    let (profile, revision) = state
        .database
        .execute(|store| {
            let repository = store.create_repository_connection(
                "repository",
                &v1::CreateRepositoryConnectionRequest {
                    idempotency_key: "repository".into(),
                    display_name: "Sandbox".into(),
                    default_branch: "main".into(),
                    provider: Some(v1::RepositoryProvider {
                        provider: Some(v1::repository_provider::Provider::Github(
                            v1::GithubRepository {
                                repository_id: "123".into(),
                                owner: "ayni-oss".into(),
                                name: "sandbox".into(),
                                installation_id: 42,
                                html_url: "https://github.com/ayni-oss/sandbox".into(),
                            },
                        )),
                    }),
                    ..Default::default()
                },
            )?;
            let snapshot = store.observe_repository_snapshot(&repository.id, &"a".repeat(40))?;
            let (profile, _) = store.create_runtime_profile(
                "profile",
                &v1::CreateRuntimeProfileRequest {
                    idempotency_key: "profile".into(),
                    repository_connection_id: repository.id,
                    repository_snapshot_id: snapshot.id,
                    configuration: Some(v1::RuntimeProfileConfiguration {
                        configuration: Some(
                            v1::runtime_profile_configuration::Configuration::AyniManaged(
                                v1::AyniManagedProfileRevision {
                                    lock_fingerprint: "c".repeat(64),
                                    ayni_version: "0.15.0".into(),
                                    oci_repository: "ghcr.io/ayni-oss/sandbox".into(),
                                    runtime_source_digest: None,
                                },
                            ),
                        ),
                    }),
                },
            )?;
            Ok((
                profile.id,
                profile
                    .active_revision_id
                    .ok_or(crate::adapters::sqlite::CleanStoreError::CorruptState)?,
            ))
        })
        .await?;
    Ok((profile, revision))
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("state.sqlite"), 3)?;
        let (profile, revision) = create_profile(&state).await?;
        let runtime = ProfileRuntime {
            image: format!("ghcr.io/ayni-oss/sandbox:ayni-arm64-{}", "c".repeat(64)),
            platform: "linux/arm64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: Some("pull-only".into()),
            build: Some(
                ayni_orchestrator_application::execution_settings::BuildRuntime {
                    configuration_revision: 0,
                    image: format!("ghcr.io/ayni-oss/ayni-builder@sha256:{}", "d".repeat(64)),
                    service_account: "runtime-builder".into(),
                    registry_secret: "push-only".into(),
                    signing_secret: "signing-only".into(),
                    engine_secret: "engine-only".into(),
                    engine_host: "tcp://isolated.test:2376".into(),
                    isolated_engine: true,
                    engine: None,
                },
            ),
        };
        let configuration = RuntimeConfiguration {
            verification_mode: crate::process::runtime_configuration::VerificationMode::Shadow,
            reporting: None,
            namespace: "test".into(),
            worker_image: format!("ghcr.io/team/worker@sha256:{}", "d".repeat(64)),
            output_claim: "outputs".into(),
            model_secret: "model".into(),
            interval_seconds: 10,
            request_timeout_seconds: 10,
            coding_turns: 24,
            quality_repair_limit: 3,
            concurrency: 4,
            allow_emulation: false,
            engine_mode: "external-isolated".into(),

            profiles: BTreeMap::new(),
            managed_runtime: None,
        };
        let cursor = RuntimeCursor {
            pending: Some(PendingVerification {
                observation: None,
                name: "verify-frozen".into(),
                profile_revision: revision,
                request: GenericRequest {
                    repository_url: "https://github.com/ayni-oss/sandbox.git".into(),
                    revision: "a".repeat(40),
                    expected: RuntimeExpectation {
                        lock_fingerprint: format!("sha256:{}", "c".repeat(64)),
                        platform: "linux/arm64".into(),
                        ayni_version: "0.15.0".into(),
                    },
                },
                image: runtime.image.clone(),
                frozen_runtime: Some(runtime.clone()),
                started_at: 1,
                source_digest: "frozen".into(),
                lock_digest: format!("sha256:{}", "c".repeat(64)),
                build_name: None,
                phase: ImagePreparationPhase::default(),
                candidate_image_id: None,
                publish_name: None,
                publish_attempts: 0,
                publish_remaining: 3,
                candidate_missing: false,
                force_rebuild: false,
                certificate: None,
                resolved_digest: None,
                verification: None,
                failed: false,
            }),
            ..Default::default()
        };
        let reconciler = RuntimeReconciler {
            state,
            configuration,
            source: Arc::new(SourceClient::unconfigured(10)?),
            kubernetes: Backend::default(),
            pass: Mutex::new(()),
            coding_pass: Mutex::new(()),
            credential_refresh: Mutex::new(BTreeMap::new()),
        };
        reconciler.save(&profile, &cursor).await?;
        Ok(Self {
            directory,
            reconciler,
            profile,
            runtime,
            cursor,
        })
    }
    async fn step(&mut self) -> Result<(), &'static str> {
        let observation = ProfileObservation {
            id: &self.profile,
            runtime: &self.runtime,
            time: 2,
            github: v1::GithubRepository::default(),
            head: "a".repeat(40),
            token: "checkout-scoped".into(),
            source_digest: "frozen".into(),
            default_branch: "main".into(),
            selected_revision: None,
            registry: v1::RuntimeRegistrySelection::default(),
            certificate: None,
        };
        Box::pin(
            self.reconciler
                .observe_pending(&observation, &mut self.cursor),
        )
        .await
    }
    fn discovery(&self, outcome: ImageDiscovery) {
        self.reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .discovery = Some(outcome);
    }
    fn complete(&self, name: &str, state: ExecutionState) {
        self.reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .insert(name.into(), state);
    }
    fn pending(&self) -> TestResult<&PendingVerification> {
        self.cursor
            .pending
            .as_ref()
            .ok_or_else(|| "pending missing".into())
    }
    async fn built(&mut self) -> TestResult {
        self.discovery(ImageDiscovery::Missing);
        self.step().await?;
        self.step().await?;
        let build = self.pending()?.build_name.clone().ok_or("build missing")?;
        self.complete(&build, ExecutionState::Succeeded);
        self.step().await?;
        self.step().await?;
        Ok(())
    }
}

#[tokio::test]
async fn missing_image_builds_once_and_restart_resumes_the_same_publication() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.built().await?;
    let publisher = fixture.pending()?.publish_name.clone().ok_or("publisher")?;
    let pending = fixture.pending()?.clone();
    fixture.reconciler.state = AppState::open(fixture.directory.path().join("state.sqlite"), 3)?;
    let profile = fixture.profile.clone();
    fixture.cursor = fixture
        .reconciler
        .state
        .database
        .execute(move |store| store.runtime_cursor(&profile))
        .await?;
    fixture.step().await?;
    assert_eq!(fixture.pending()?.publish_name, Some(publisher.clone()));
    assert_eq!(
        fixture.pending()?.request.revision,
        pending.request.revision
    );
    assert_eq!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .builds
            .len(),
        1
    );
    fixture.complete(&publisher, ExecutionState::Failed);
    fixture.step().await?;
    fixture.discovery(ImageDiscovery::Resolved {
        digest: format!("sha256:{}", "e".repeat(64)),
        image_id: format!("sha256:{}", "b".repeat(64)),
    });
    fixture.step().await?;
    assert!(fixture.pending()?.resolved_digest.is_some());
    assert_eq!(
        fixture.cursor.status.get("phase").map(String::as_str),
        Some("verifying")
    );
    assert_eq!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .builds
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn publication_retries_same_candidate_and_rebuilds_evicted_candidate() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.built().await?;
    let candidate = fixture.pending()?.candidate_image_id.clone();
    let original_build = fixture.pending()?.build_name.clone();
    let publisher = fixture.pending()?.publish_name.clone().ok_or("publisher")?;
    fixture.complete(&publisher, ExecutionState::Failed);
    fixture.step().await?;
    fixture.step().await?; // authenticated missing-image read-back
    fixture.step().await?; // publication of the same certified candidate
    assert_eq!(fixture.pending()?.candidate_image_id, candidate);
    assert_eq!(fixture.pending()?.build_name, original_build);
    let publisher = fixture.pending()?.publish_name.clone().ok_or("publisher")?;
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .candidate_missing = true;
    fixture.complete(&publisher, ExecutionState::Failed);
    fixture.step().await?;
    fixture.step().await?;
    assert!(fixture.pending()?.phase != ImagePreparationPhase::Publishing);
    assert_ne!(fixture.pending()?.build_name, original_build);
    assert_eq!(fixture.pending()?.request.revision, "a".repeat(40));
    Ok(())
}

#[tokio::test]
async fn registry_errors_and_exhausted_retries_never_authorize_a_new_build() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.discovery(ImageDiscovery::Failed {
        reason: "registry_access_denied".into(),
    });
    assert_eq!(fixture.step().await, Err("registry_access_denied"));
    assert!(fixture.pending()?.build_name.is_none());
    fixture.built().await?;
    for _ in 0..3 {
        let publisher = fixture.pending()?.publish_name.clone().ok_or("publisher")?;
        fixture.complete(&publisher, ExecutionState::Failed);
        fixture.step().await?;
        if fixture.pending()?.publish_remaining == 0 {
            assert_eq!(fixture.step().await, Err("runtime_publication_failed"));
            break;
        }
        fixture.step().await?;
        fixture.step().await?;
    }
    assert_eq!(fixture.pending()?.publish_remaining, 0);
    assert_eq!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .builds
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn existing_image_enters_verification_and_explicit_rebuild_handles_platform_mismatch()
-> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.discovery(ImageDiscovery::Resolved {
        digest: format!("sha256:{}", "e".repeat(64)),
        image_id: format!("sha256:{}", "b".repeat(64)),
    });
    fixture.step().await?;
    assert!(fixture.pending()?.build_name.is_none());
    assert!(fixture.pending()?.resolved_digest.is_some());
    let mut fixture = Fixture::new().await?;
    fixture.discovery(ImageDiscovery::Failed {
        reason: "runtime_platform_mismatch".into(),
    });
    assert_eq!(fixture.step().await, Err("runtime_platform_mismatch"));
    fixture
        .cursor
        .pending
        .as_mut()
        .ok_or("pending")?
        .force_rebuild = true;
    fixture.step().await?;
    assert!(fixture.pending()?.build_name.is_some());
    Ok(())
}

#[tokio::test]
async fn interrupted_publisher_probes_registry_before_reissuing_the_push() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.built().await?;
    let publisher = fixture.pending()?.publish_name.clone().ok_or("publisher")?;
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .states
        .remove(&publisher);
    fixture.step().await?;
    assert!(fixture.pending()?.phase.discovering());
    fixture.discovery(ImageDiscovery::Resolved {
        digest: format!("sha256:{}", "e".repeat(64)),
        image_id: format!("sha256:{}", "b".repeat(64)),
    });
    fixture.step().await?;
    assert!(fixture.pending()?.resolved_digest.is_some());
    assert_eq!(
        fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .publications
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn exhausted_operations_persist_failure_and_clean_jobs_without_restarting() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.built().await?;
    for _ in 0..3 {
        fixture
            .reconciler
            .record_failure(&fixture.profile, "runtime_publication_failed")
            .await?;
    }
    let id = fixture.profile.clone();
    fixture.cursor = fixture
        .reconciler
        .with_database(move |store| {
            store
                .runtime_cursor(&id)
                .map_err(|_| "runtime_cursor_invalid")
        })
        .await?;
    assert!(fixture.pending()?.failed);
    assert_eq!(
        fixture.cursor.status.get("reason-code").map(String::as_str),
        Some("runtime_publication_failed")
    );
    fixture.step().await?;
    let jobs = fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(jobs.states.is_empty());
    assert_eq!(jobs.builds.len(), 1);
    assert_eq!(jobs.publications.len(), 1);
    drop(jobs);
    Ok(())
}

#[tokio::test]
async fn explicit_refresh_discards_old_intent_without_counting_a_preparation_failure() -> TestResult
{
    let mut fixture = Fixture::new().await?;
    fixture
        .cursor
        .pending
        .as_mut()
        .ok_or("pending")?
        .source_digest
        .clear();
    fixture.step().await?;
    assert!(fixture.cursor.pending.is_none());
    assert_eq!(fixture.cursor.failures, 0);
    assert!(!fixture.cursor.status.contains_key("reason-code"));
    assert_eq!(
        fixture.cursor.status.get("phase").map(String::as_str),
        Some("analyzing")
    );
    Ok(())
}

#[tokio::test]
async fn engine_loss_discards_candidate_and_restart_preserves_fresh_attempt() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.built().await?;
    let old = fixture.pending()?.clone();
    let build = old.build_name.clone().ok_or("build")?;
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .lost_engines
        .insert(build.clone());
    fixture.step().await?;
    let fresh = fixture.pending()?.clone();
    assert_ne!(fresh.build_name, old.build_name);
    assert!(fresh.candidate_image_id.is_none());
    assert!(fresh.publish_name.is_none());
    assert!(fresh.phase == ImagePreparationPhase::Building);
    assert_eq!(fresh.request.revision, old.request.revision);
    assert_eq!(fresh.publish_remaining, old.publish_remaining - 1);
    fixture.reconciler.state = AppState::open(fixture.directory.path().join("state.sqlite"), 3)?;
    let profile = fixture.profile.clone();
    fixture.cursor = fixture
        .reconciler
        .state
        .database
        .execute(move |s| s.runtime_cursor(&profile))
        .await?;
    fixture.step().await?;
    assert_eq!(fixture.pending()?.build_name, fresh.build_name);
    assert!(fixture.pending()?.phase == ImagePreparationPhase::BuildingStarted);
    let jobs = fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(!jobs.states.contains_key(&build));
    assert!(
        !jobs
            .states
            .contains_key(old.publish_name.as_deref().ok_or("publish")?)
    );
    drop(jobs);
    Ok(())
}

#[tokio::test]
async fn builder_resolution_freezes_manifest_before_restartable_cleanup() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture.cursor.pending = None;
    fixture.cursor.preparation = Some(PreparationIdentity {
        operation_reporting: None,
        operation_observations: std::collections::BTreeMap::new(),
        verification_authority:
            crate::profiles::ayni::preparation_state::VerificationAuthority::Retained,
        generation: "approved-generation".into(),
        source_revision: "a".repeat(40),
        policy_digest: "policy".into(),
        lock_digest: "lock".into(),
        contract_version: "0.15.0".into(),
        platform: fixture.runtime.platform.clone(),
        registry: vec![],
        certificate: None,
        runtime: fixture.runtime.clone(),
    });
    fixture.discovery(ImageDiscovery::Resolved {
        digest: format!("sha256:{}", "a".repeat(64)),
        image_id: String::new(),
    });
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cleanup_pending = true;
    assert!(
        fixture
            .reconciler
            .resolve_builder(
                &fixture.profile,
                "0.15.0",
                &mut fixture.runtime,
                &mut fixture.cursor
            )
            .await
            .is_err()
    );
    fixture
        .reconciler
        .record_failure(&fixture.profile, "runtime_cleanup_pending")
        .await?;
    let id = fixture.profile.clone();
    fixture.cursor = fixture
        .reconciler
        .state
        .database
        .execute(move |store| store.runtime_cursor(&id))
        .await?;
    fixture
        .reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cleanup_pending = false;
    assert_eq!(fixture.cursor.failures, 0);
    assert!(!fixture.cursor.status.contains_key("reason-code"));
    fixture.discovery(ImageDiscovery::Resolved {
        digest: format!("sha256:{}", "b".repeat(64)),
        image_id: String::new(),
    });
    assert!(
        fixture
            .reconciler
            .resolve_builder(
                &fixture.profile,
                "0.15.0",
                &mut fixture.runtime,
                &mut fixture.cursor
            )
            .await?
    );
    assert_eq!(
        fixture
            .runtime
            .build
            .as_ref()
            .ok_or("builder missing")?
            .image,
        format!("ghcr.io/ayni-oss/ayni-builder@sha256:{}", "a".repeat(64))
    );
    assert!(!fixture.cursor.status.contains_key("builder-job"));
    Ok(())
}

#[test]
fn retry_never_grants_permission_to_replace_a_runtime() {
    let mut cursor = RuntimeCursor {
        failures: 3,
        next_at: 99,
        ..Default::default()
    };
    cursor.request_rebuild(false);
    assert!(!cursor.force_rebuild);
    assert!(cursor.retry_trust);
    cursor.retry();
    assert_eq!(cursor.failures, 0);
    assert_eq!(cursor.next_at, 0);
    assert!(!cursor.force_rebuild);
    cursor.request_rebuild(true);
    assert!(cursor.force_rebuild);
}

#[tokio::test]
async fn cleanup_wait_preserves_failure_budget_and_frozen_operation() -> TestResult {
    let fixture = Fixture::new().await?;
    let id = fixture.profile.clone();
    let mut cursor = fixture.cursor.clone();
    cursor.failures = 1;
    cursor
        .status
        .insert("reason-code".into(), "runtime_publication_failed".into());
    fixture.reconciler.save(&id, &cursor).await?;
    let frozen = serde_json::to_value(&cursor.pending)?;
    for _ in 0..4 {
        fixture
            .reconciler
            .record_failure(&id, "runtime_cleanup_pending")
            .await?;
    }
    let stored = fixture
        .reconciler
        .state
        .database
        .execute(move |store| store.runtime_cursor(&id))
        .await?;
    assert_eq!(stored.failures, 1);
    assert_eq!(
        stored.status.get("reason-code").map(String::as_str),
        Some("runtime_publication_failed")
    );
    assert_eq!(
        stored.status.get("phase").map(String::as_str),
        Some("cleaning_up")
    );
    assert!(stored.next_at <= now()? + 1);
    assert_eq!(serde_json::to_value(&stored.pending)?, frozen);
    Ok(())
}

#[tokio::test]
async fn cleanup_wait_does_not_degrade_profile() -> TestResult {
    let fixture = Fixture::new().await?;
    fixture
        .reconciler
        .record_failure(&fixture.profile, "runtime_cleanup_pending")
        .await?;
    let id = fixture.profile.clone();
    let (cursor, profile) = fixture
        .reconciler
        .state
        .database
        .execute(move |store| Ok((store.runtime_cursor(&id)?, store.runtime_profile(&id)?)))
        .await?;
    assert_eq!(cursor.failures, 0);
    assert!(!cursor.status.contains_key("reason-code"));
    assert!(profile.failure.is_none());
    Ok(())
}

#[test]
fn persisted_cleanup_failure_recovers_without_authorizing_rebuild() {
    let mut cursor = RuntimeCursor {
        failures: 3,
        next_at: u64::MAX,
        ..RuntimeCursor::default()
    };
    cursor
        .status
        .insert("reason-code".into(), "runtime_cleanup_pending".into());
    cursor
        .status
        .insert("builder-image".into(), "frozen-builder".into());
    assert!(cursor.resume_cleanup());
    assert_eq!(cursor.failures, 3);
    assert_eq!(cursor.next_at, 0);
    assert!(!cursor.force_rebuild);
    assert_eq!(
        cursor.status.get("builder-image").map(String::as_str),
        Some("frozen-builder")
    );
    assert!(!cursor.resume_cleanup());
    cursor
        .status
        .insert("reason-code".into(), "runtime_cleanup_failed".into());
    cursor.failures = 3;
    assert!(!cursor.resume_cleanup());
    assert_eq!(cursor.failures, 3);
}

#[tokio::test]
async fn repository_removal_cleanup_retries_after_restart_and_releases_credentials() -> TestResult {
    let mut fixture = Fixture::new().await?;
    fixture
        .cursor
        .pending
        .as_mut()
        .ok_or("pending operation missing")?
        .failed = true;
    fixture.cursor.failures = 3;
    fixture
        .reconciler
        .save(&fixture.profile, &fixture.cursor)
        .await?;
    let profile_id = fixture.profile.clone();
    fixture
        .reconciler
        .state
        .database
        .execute(move |store| {
            let profile = store.runtime_profile(&profile_id)?;
            let repository = store.repository_connection(&profile.repository_connection_id)?;
            store.remove_repository(&v1::RemoveRepositoryRequest {
                idempotency_key: "remove-repository".into(),
                repository_connection_id: repository.id,
                expected_resource_version: repository.resource_version,
            })
        })
        .await?;
    // Frozen preparation credentials remain retained until all owned operations are cleaned.
    assert!(
        fixture
            .reconciler
            .state
            .database
            .execute(|store| store.connection_secret_in_use("pull-only"))
            .await?
    );
    fixture.reconciler.state.shutdown_database().await?;
    fixture.reconciler.state = AppState::open(fixture.directory.path().join("state.sqlite"), 3)?;
    {
        let mut jobs = fixture
            .reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.cleanup_pending = true;
        jobs.states
            .insert("verify-frozen".into(), ExecutionState::Running);
    }
    let reconciler = Arc::new(fixture.reconciler);
    reconciler.reconcile().await;
    assert_eq!(
        reconciler
            .state
            .database
            .execute(|store| store.removed_runtime_cleanup().map(|cursors| cursors.len()))
            .await?,
        1
    );
    reconciler
        .kubernetes
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cleanup_pending = false;
    reconciler.reconcile().await;
    assert!(
        reconciler
            .state
            .database
            .execute(|store| store.removed_runtime_cleanup())
            .await?
            .is_empty()
    );
    assert!(
        !reconciler
            .state
            .database
            .execute(|store| store.connection_secret_in_use("pull-only"))
            .await?
    );
    assert!(
        !reconciler
            .kubernetes
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .states
            .contains_key("verify-frozen")
    );
    Ok(())
}

mod retries;

mod cutover;
