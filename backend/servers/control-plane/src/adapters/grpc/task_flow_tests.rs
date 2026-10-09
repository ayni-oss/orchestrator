//! Exercises the native transport and durable lifecycle together. Source,
//! environment and worker observations are fixtures, not provider execution.

use super::{AppState, Code, RichStatus, Status, serve, v1};
use prost::Message;
use std::{path::Path, time::Duration};
use tokio::{sync::watch as shutdown_watch, task::JoinHandle};
use tonic::{Streaming, transport::Channel};
use v1::{
    development_task_service_client::DevelopmentTaskServiceClient,
    repository_service_client::RepositoryServiceClient,
    runtime_profile_service_client::RuntimeProfileServiceClient,
    watch_development_task_response::Update,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type TaskClient = DevelopmentTaskServiceClient<Channel>;
const SOURCE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IMAGE: &str = "ghcr.io/test-org/repository-ayni";

struct Server {
    state: AppState,
    shutdown: shutdown_watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl Server {
    async fn start(path: &Path) -> TestResult<(Self, Channel)> {
        let state = AppState::open(path, 3)?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        drop(listener);
        let (shutdown, receiver) = shutdown_watch::channel(false);
        let task = tokio::spawn(serve(address, state.clone(), receiver));
        let server = Self {
            state,
            shutdown,
            task: Some(task),
        };
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?
            .timeout(Duration::from_secs(3));
        let channel = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(channel) = endpoint.connect().await {
                    break channel;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        Ok((server, channel))
    }

    async fn stop(mut self) -> TestResult {
        self.shutdown.send(true)?;
        let mut task = self.task.take().ok_or("server task missing")?;
        let result = tokio::time::timeout(Duration::from_secs(3), &mut task).await;
        if result.is_err() {
            task.abort();
        }
        result???;
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

struct RuntimeFixture {
    project: v1::Project,
    repository: v1::RepositoryConnection,
    snapshot: v1::RepositorySnapshot,
    profile: v1::RuntimeProfile,
    revision: v1::RuntimeProfileRevision,
}

async fn create_profile(server: &Server, channel: Channel) -> TestResult<RuntimeFixture> {
    let repository = RepositoryServiceClient::new(channel.clone())
        .create_repository_connection(v1::CreateRepositoryConnectionRequest {
            idempotency_key: "repository".into(),
            display_name: "Task flow fixture".into(),
            default_branch: "main".into(),
            provider: Some(v1::RepositoryProvider {
                provider: Some(v1::repository_provider::Provider::Github(
                    v1::GithubRepository {
                        repository_id: "123".into(),
                        owner: "test-org".into(),
                        name: "repository".into(),
                        installation_id: 42,
                        html_url: "https://github.com/test-org/repository".into(),
                    },
                )),
            }),
            registry_connection_id: String::new(),
            image_namespace: String::new(),
        })
        .await?
        .into_inner()
        .repository_connection
        .ok_or("repository missing")?;
    let project = v1::project_service_client::ProjectServiceClient::new(channel.clone())
        .create_project(v1::CreateProjectRequest {
            idempotency_key: "project".into(),
            display_name: "Task flow project".into(),
            repository_connection_id: repository.id.clone(),
            repository_connection_ids: vec![],
        })
        .await?
        .into_inner()
        .project
        .ok_or("project missing")?;
    // The source observer normally records this immutable provider observation.
    let repository_id = repository.id.clone();
    let snapshot = with_store(&server.state, move |store| {
        store.observe_repository_snapshot(&repository_id, SOURCE)
    })
    .await?;
    let created = RuntimeProfileServiceClient::new(channel)
        .create_runtime_profile(v1::CreateRuntimeProfileRequest {
            idempotency_key: "profile".into(),
            repository_connection_id: repository.id.clone(),
            repository_snapshot_id: snapshot.id.clone(),
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: Some(
                    v1::runtime_profile_configuration::Configuration::AyniManaged(
                        v1::AyniManagedProfileRevision {
                            lock_fingerprint: "c".repeat(64),
                            ayni_version: "0.15.0".into(),
                            oci_repository: IMAGE.into(),
                            runtime_source_digest: None,
                        },
                    ),
                ),
            }),
        })
        .await?
        .into_inner();
    Ok(RuntimeFixture {
        project,
        repository,
        snapshot,
        profile: created.runtime_profile.ok_or("profile missing")?,
        revision: created.revision.ok_or("profile revision missing")?,
    })
}

async fn observe_verified_runtime(
    server: &Server,
    profile: &RuntimeFixture,
    digest: &str,
) -> TestResult<v1::RuntimeArtifact> {
    // Inject the trusted verifier's observation at the durable boundary; the
    // test does not execute Ayni or establish real registry/certificate trust.
    let profile_id = profile.profile.id.clone();
    let revision_id = profile.revision.id.clone();
    let digest = digest.to_owned();
    Ok(with_store(&server.state, move |store| {
        store.record_runtime_artifact(
            &profile_id,
            v1::RuntimeArtifact {
                runtime_profile_revision_id: revision_id,
                oci_repository: IMAGE.into(),
                resolved_digest: digest,
                platform: Some(v1::OciPlatform {
                    operating_system: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                }),
                status: v1::RuntimeArtifactStatus::Verified as i32,
                verified_metadata: vec![
                    v1::VerifiedMetadata {
                        key: "ayni-contract".into(),
                        value: "0.15.0".into(),
                    },
                    v1::VerifiedMetadata {
                        key: "builder-image".into(),
                        value: format!("ghcr.io/ayni-oss/ayni-builder@sha256:{}", "a".repeat(64)),
                    },
                ],
                ..Default::default()
            },
        )
    })
    .await?)
}

fn task_request(profile: &RuntimeFixture, key: &str) -> v1::CreateDevelopmentTaskRequest {
    v1::CreateDevelopmentTaskRequest {
        idempotency_key: key.into(),
        request: Some(v1::DevelopmentTaskRequest {
            project_id: profile.project.id.clone(),
            repository_connection_id: profile.repository.id.clone(),
            repository_revision: SOURCE.into(),
            runtime_profile_id: profile.profile.id.clone(),
            agent: Some(v1::AgentSelection {
                provider: "openai".into(),
                agent: "coding-agent".into(),
                model: "gpt-5".into(),
            }),
            request: "Update the documented example".into(),
            acceptance_criteria: vec!["The example matches current behavior".into()],
            change_constraints: Some(v1::ChangeConstraints {
                protected_paths: vec!["policy".into()],
                allow_test_edits: false,
            }),
            deadline: Some(prost_types::Duration {
                seconds: 3_600,
                nanos: 0,
            }),
        }),
    }
}

fn assert_error<T>(
    result: Result<tonic::Response<T>, Status>,
    code: Code,
    reason: v1::ErrorReason,
) -> TestResult {
    let error = result
        .err()
        .ok_or("expected the RPC to reject the request")?;
    assert_eq!(error.code(), code);
    let rich = RichStatus::decode(error.details())?;
    let detail = rich.details.first().ok_or("error detail missing")?;
    let detail = v1::ErrorDetail::decode(detail.value.as_slice())?;
    assert_eq!(detail.reason, reason as i32);
    assert!(error.metadata().contains_key("x-request-id"));
    Ok(())
}

async fn get_task(client: &mut TaskClient, id: &str) -> TestResult<v1::DevelopmentTask> {
    Ok(client
        .get_development_task(v1::GetDevelopmentTaskRequest {
            development_task_id: id.into(),
        })
        .await?
        .into_inner()
        .development_task
        .ok_or("task missing")?)
}

async fn next_update(
    stream: &mut Streaming<v1::WatchDevelopmentTaskResponse>,
) -> TestResult<v1::WatchDevelopmentTaskResponse> {
    Ok(
        tokio::time::timeout(Duration::from_secs(3), stream.message())
            .await??
            .ok_or("watch ended before expected event")?,
    )
}

fn assert_frozen_inputs(
    profile: &RuntimeFixture,
    artifact: &v1::RuntimeArtifact,
    request: &v1::CreateDevelopmentTaskRequest,
    task: &v1::DevelopmentTask,
    attempt: &v1::DevelopmentAttempt,
) -> TestResult {
    let frozen = task.snapshot.as_ref().ok_or("snapshot missing")?;
    assert_eq!(attempt.snapshot, task.snapshot);
    assert_eq!(
        frozen
            .repository
            .as_ref()
            .ok_or("repository missing")?
            .repository_snapshot_id,
        profile.snapshot.id
    );
    let runtime = frozen.runtime.as_ref().ok_or("runtime missing")?;
    assert_eq!(runtime.runtime_profile_revision_id, profile.revision.id);
    assert_eq!(runtime.runtime_artifact_id, artifact.id);
    assert_eq!(runtime.resolved_digest, artifact.resolved_digest);
    assert_eq!(runtime.platform, artifact.platform);
    assert_eq!(
        frozen.agent,
        request.request.as_ref().ok_or("request missing")?.agent
    );
    assert_eq!(
        frozen.change_constraints,
        request
            .request
            .as_ref()
            .ok_or("request missing")?
            .change_constraints
    );
    assert!(frozen.deadline_at.is_some());
    Ok(())
}

mod lifecycle;

use crate::coordinators::with_store_async as with_store;
