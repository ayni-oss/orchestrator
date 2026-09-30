//! Native gRPC adapters for the durable clean-v1 product contract.

use crate::{
    AppState,
    application_metrics::record_grpc_completion,
    clean_store::{
        CleanStoreError, DEVELOPMENT_SERVICE, PROFILE_SERVICE, REPOSITORY_SERVICE, REVIEW_SERVICE,
        StoredEvent,
    },
    grpc_access::{
        ClientIdentity, CustomClientAccess, CustomClientGuard,
        MAX_CONCURRENT_STREAMS_PER_CONNECTION, MAX_HTTP2_HEADER_LIST_BYTES,
    },
    with_store,
};
use ayni_orchestrator_api::{FILE_DESCRIPTOR_SET, v1, validation as api_validation};
use prost::Message;
use prost_types::{Any, Timestamp};
use sha2::{Digest as _, Sha256};
use std::{io, net::SocketAddr, pin::Pin, str::FromStr, time::Instant};
use tokio::sync::watch;
use tokio_stream::Stream;
use tonic::{
    Code, Request, Response, Status,
    metadata::MetadataValue,
    transport::{Identity, Server, ServerTlsConfig},
};
use tracing::{info, warn};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail";
type BoxGrpcStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

#[derive(Clone)]
pub struct GrpcApi {
    state: AppState,
    custom_clients: Option<CustomClientGuard>,
    shutdown: watch::Receiver<bool>,
}

impl GrpcApi {
    #[must_use]
    pub(crate) const fn new(state: AppState, shutdown: watch::Receiver<bool>) -> Self {
        Self {
            state,
            custom_clients: None,
            shutdown,
        }
    }

    const fn new_custom(
        state: AppState,
        custom_clients: CustomClientGuard,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            state,
            custom_clients: Some(custom_clients),
            shutdown,
        }
    }

    fn context<T>(
        &self,
        request: &Request<T>,
        rpc: &'static str,
    ) -> Result<RequestContext, Status> {
        let client = self
            .custom_clients
            .as_ref()
            .map(|guard| guard.authorize(request, rpc))
            .transpose()?;
        Ok(RequestContext::from_request(request, rpc, client.as_ref()))
    }
}

/// Serves the canonical local gRPC API, health service, and reflection.
///
/// # Errors
///
/// Returns an I/O error when the listener cannot be served.
pub async fn serve(
    address: SocketAddr,
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), io::Error> {
    let api = GrpcApi::new(state, shutdown.clone());
    let (health_reporter, health) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<v1::repository_service_server::RepositoryServiceServer<GrpcApi>>()
        .await;
    health_reporter
        .set_serving::<v1::runtime_profile_service_server::RuntimeProfileServiceServer<GrpcApi>>()
        .await;
    health_reporter
        .set_serving::<v1::provider_service_server::ProviderServiceServer<GrpcApi>>()
        .await;
    health_reporter
        .set_serving::<v1::development_task_service_server::DevelopmentTaskServiceServer<GrpcApi>>()
        .await;
    health_reporter
        .set_serving::<v1::review_service_server::ReviewServiceServer<GrpcApi>>()
        .await;
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()
        .map_err(io::Error::other)?;

    info!(address = %address, "gRPC control plane started");
    Server::builder()
        .add_service(health)
        .add_service(reflection)
        .add_service(repository_server(api.clone()))
        .add_service(profile_server(api.clone()))
        .add_service(provider_server(api.clone()))
        .add_service(development_server(api.clone()))
        .add_service(review_server(api))
        .serve_with_shutdown(address, async move {
            if !*shutdown.borrow() {
                let _ = shutdown.changed().await;
            }
        })
        .await
        .map_err(io::Error::other)
}

/// Serves the capability-authorized TLS custom-client gRPC API.
///
/// # Errors
///
/// Returns an I/O error when TLS setup or serving fails.
pub async fn serve_custom(
    address: SocketAddr,
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
    access: CustomClientAccess,
) -> Result<(), io::Error> {
    let api = GrpcApi::new_custom(state, access.guard(), shutdown.clone());
    let tls = ServerTlsConfig::new()
        .identity(Identity::from_pem(
            access.certificate_pem(),
            access.private_key_pem(),
        ))
        .timeout(std::time::Duration::from_secs(10));
    let mut server = Server::builder()
        .tls_config(tls)
        .map_err(io::Error::other)?
        .http2_max_header_list_size(Some(MAX_HTTP2_HEADER_LIST_BYTES))
        .max_concurrent_streams(MAX_CONCURRENT_STREAMS_PER_CONNECTION)
        .concurrency_limit_per_connection(
            usize::try_from(MAX_CONCURRENT_STREAMS_PER_CONNECTION).unwrap_or(32),
        )
        .load_shed(true);
    info!(address = %address, tls = true, authentication = "bearer", reflection = false,
        "custom-client gRPC control plane started");
    server
        .add_service(repository_server(api.clone()))
        .add_service(profile_server(api.clone()))
        .add_service(provider_server(api.clone()))
        .add_service(development_server(api.clone()))
        .add_service(review_server(api))
        .serve_with_shutdown(address, async move {
            if !*shutdown.borrow() {
                let _ = shutdown.changed().await;
            }
        })
        .await
        .map_err(io::Error::other)
}

fn repository_server(
    api: GrpcApi,
) -> v1::repository_service_server::RepositoryServiceServer<GrpcApi> {
    v1::repository_service_server::RepositoryServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES)
}
fn profile_server(
    api: GrpcApi,
) -> v1::runtime_profile_service_server::RuntimeProfileServiceServer<GrpcApi> {
    v1::runtime_profile_service_server::RuntimeProfileServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES)
}
fn provider_server(api: GrpcApi) -> v1::provider_service_server::ProviderServiceServer<GrpcApi> {
    v1::provider_service_server::ProviderServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES)
}
fn development_server(
    api: GrpcApi,
) -> v1::development_task_service_server::DevelopmentTaskServiceServer<GrpcApi> {
    v1::development_task_service_server::DevelopmentTaskServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES)
}
fn review_server(api: GrpcApi) -> v1::review_service_server::ReviewServiceServer<GrpcApi> {
    v1::review_service_server::ReviewServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES)
}

struct RequestContext {
    request_id: String,
    client_id: Option<String>,
    rpc: &'static str,
    started: Instant,
}

impl RequestContext {
    fn from_request<T>(
        request: &Request<T>,
        rpc: &'static str,
        client: Option<&ClientIdentity>,
    ) -> Self {
        let request_id = client.and_then(ClientIdentity::request_id).map_or_else(
            || {
                request
                    .metadata()
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .filter(|value| valid_request_id(value))
                    .map_or_else(|| Uuid::now_v7().to_string(), str::to_owned)
            },
            str::to_owned,
        );
        Self {
            request_id,
            client_id: client.map(|identity| identity.id().to_owned()),
            rpc,
            started: Instant::now(),
        }
    }

    fn response<T>(&self, value: T) -> Response<T> {
        record_grpc_completion(self.rpc, Code::Ok);
        let mut response = Response::new(value);
        self.add_request_id(response.metadata_mut());
        info!(request_id = %self.request_id,
            client_id = self.client_id.as_deref().unwrap_or("trusted-local"), rpc = self.rpc,
            status = "ok", latency_ms = elapsed_millis(self.started), "gRPC request completed");
        response
    }

    fn idempotency_key(&self, key: &str) -> Result<String, Status> {
        api_validation::idempotency_key(key).map_err(|error| self.validation(&error))?;
        let scope = self.client_id.as_deref().unwrap_or("trusted-local");
        let digest =
            Sha256::digest(format!("ayni.idempotency.v1\0{scope}\0{}\0{key}", self.rpc).as_bytes());
        Ok(format!("request-{digest:x}"))
    }

    fn validation(&self, error: &api_validation::ValidationError) -> Status {
        self.status(
            Code::InvalidArgument,
            error.reason,
            "the request violates the documented contract",
            None,
            None,
            Some(("field", error.field)),
        )
    }

    fn unsupported_profile(&self) -> Status {
        self.status(
            Code::FailedPrecondition,
            v1::ErrorReason::ProfileUnsupported,
            "Custom Runtime is modeled but is not operational in this release",
            None,
            None,
            None,
        )
    }

    fn store_error(&self, error: &CleanStoreError) -> Status {
        let (code, reason, resource_type, resource_id) = match error {
            CleanStoreError::NotFound {
                resource_type,
                resource_id,
            } => (
                Code::NotFound,
                v1::ErrorReason::ResourceNotFound,
                Some(*resource_type),
                Some(resource_id.as_str()),
            ),
            CleanStoreError::IdempotencyConflict => (
                Code::AlreadyExists,
                v1::ErrorReason::IdempotencyKeyReused,
                None,
                None,
            ),
            CleanStoreError::StaleVersion => (Code::Aborted, v1::ErrorReason::Conflict, None, None),
            CleanStoreError::ParentMismatch | CleanStoreError::InvalidTransition => (
                Code::FailedPrecondition,
                v1::ErrorReason::FailedPrecondition,
                None,
                None,
            ),
            CleanStoreError::ProfileNotReady => (
                Code::FailedPrecondition,
                v1::ErrorReason::ProfileNotReady,
                None,
                None,
            ),
            CleanStoreError::TaskNotActionable => (
                Code::FailedPrecondition,
                v1::ErrorReason::TaskNotActionable,
                None,
                None,
            ),
            CleanStoreError::DeadlineExpired => (
                Code::FailedPrecondition,
                v1::ErrorReason::TaskDeadlineExpired,
                None,
                None,
            ),
            CleanStoreError::InvalidPageToken => (
                Code::InvalidArgument,
                v1::ErrorReason::InvalidInput,
                None,
                None,
            ),
            CleanStoreError::BoundExceeded => (
                Code::ResourceExhausted,
                v1::ErrorReason::BoundExceeded,
                None,
                None,
            ),
            CleanStoreError::WatchCursorExpired => (
                Code::OutOfRange,
                v1::ErrorReason::WatchCursorExpired,
                None,
                None,
            ),
            CleanStoreError::RetiredPilotSchema
            | CleanStoreError::UnsupportedSchemaVersion(_)
            | CleanStoreError::CreateDirectory(_)
            | CleanStoreError::Database(_)
            | CleanStoreError::CorruptState
            | CleanStoreError::Clock => (Code::Internal, v1::ErrorReason::Internal, None, None),
        };
        self.status(
            code,
            reason,
            &crate::truncate_utf8(&error.to_string(), 512),
            resource_type,
            resource_id,
            None,
        )
    }

    fn status(
        &self,
        code: Code,
        reason: v1::ErrorReason,
        message: &str,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
        metadata: Option<(&str, &str)>,
    ) -> Status {
        record_grpc_completion(self.rpc, code);
        warn!(request_id = %self.request_id,
            client_id = self.client_id.as_deref().unwrap_or("trusted-local"), rpc = self.rpc,
            status = ?code, latency_ms = elapsed_millis(self.started), "gRPC request failed");
        let detail = v1::ErrorDetail {
            reason: reason as i32,
            message: message.to_owned(),
            resource_type: resource_type.map(str::to_owned),
            resource_id: resource_id.map(|value| crate::truncate_utf8(value, 512)),
            metadata: metadata
                .map(|(key, value)| v1::ErrorMetadata {
                    key: key.to_owned(),
                    value: value.to_owned(),
                })
                .into_iter()
                .collect(),
            request_id: Some(self.request_id.clone()),
        };
        let rich = RichStatus {
            code: code as i32,
            message: message.to_owned(),
            details: vec![Any {
                type_url: ERROR_DETAIL_TYPE_URL.to_owned(),
                value: detail.encode_to_vec(),
            }],
        };
        let mut status = Status::with_details(code, message, rich.encode_to_vec().into());
        self.add_request_id(status.metadata_mut());
        status
    }

    fn add_request_id(&self, metadata: &mut tonic::metadata::MetadataMap) {
        if let Ok(value) = MetadataValue::from_str(&self.request_id) {
            metadata.insert("x-request-id", value);
        }
    }
}

#[tonic::async_trait]
impl v1::repository_service_server::RepositoryService for GrpcApi {
    async fn list_repository_connections(
        &self,
        request: Request<v1::ListRepositoryConnectionsRequest>,
    ) -> Result<Response<v1::ListRepositoryConnectionsResponse>, Status> {
        let context = self.context(&request, "ListRepositoryConnections")?;
        let message = request.into_inner();
        api_validation::enum_filter(
            message.status,
            "status",
            &[
                v1::RepositoryConnectionStatus::Connecting as i32,
                v1::RepositoryConnectionStatus::Connected as i32,
                v1::RepositoryConnectionStatus::Degraded as i32,
                v1::RepositoryConnectionStatus::Revoked as i32,
                v1::RepositoryConnectionStatus::Failed as i32,
            ],
        )
        .and_then(|()| api_validation::page(message.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let page = with_store(&self.state, |store| {
            store.list_repository_connections(message.status, message.page.as_ref())
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRepositoryConnectionsResponse {
            repository_connections: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_repository_connection(
        &self,
        request: Request<v1::GetRepositoryConnectionRequest>,
    ) -> Result<Response<v1::GetRepositoryConnectionResponse>, Status> {
        let context = self.context(&request, "GetRepositoryConnection")?;
        api_validation::resource_id(
            &request.get_ref().repository_connection_id,
            "repository_connection_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.repository_connection(&request.get_ref().repository_connection_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRepositoryConnectionResponse {
            repository_connection: Some(resource),
        }))
    }

    async fn create_repository_connection(
        &self,
        request: Request<v1::CreateRepositoryConnectionRequest>,
    ) -> Result<Response<v1::CreateRepositoryConnectionResponse>, Status> {
        let context = self.context(&request, "CreateRepositoryConnection")?;
        api_validation::create_repository_connection(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let resource = with_store(&self.state, |store| {
            store.create_repository_connection(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CreateRepositoryConnectionResponse {
            repository_connection: Some(resource),
        }))
    }

    async fn list_repository_snapshots(
        &self,
        request: Request<v1::ListRepositorySnapshotsRequest>,
    ) -> Result<Response<v1::ListRepositorySnapshotsResponse>, Status> {
        let context = self.context(&request, "ListRepositorySnapshots")?;
        validate_id(
            &request.get_ref().repository_connection_id,
            "repository_connection_id",
        )
        .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store
                .list_repository_snapshots(&message.repository_connection_id, message.page.as_ref())
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRepositorySnapshotsResponse {
            repository_snapshots: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_repository_snapshot(
        &self,
        request: Request<v1::GetRepositorySnapshotRequest>,
    ) -> Result<Response<v1::GetRepositorySnapshotResponse>, Status> {
        let context = self.context(&request, "GetRepositorySnapshot")?;
        validate_id(
            &request.get_ref().repository_snapshot_id,
            "repository_snapshot_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.repository_snapshot(&request.get_ref().repository_snapshot_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRepositorySnapshotResponse {
            repository_snapshot: Some(resource),
        }))
    }

    type WatchRepositoriesStream = BoxGrpcStream<v1::WatchRepositoriesResponse>;

    async fn watch_repositories(
        &self,
        request: Request<v1::WatchRepositoriesRequest>,
    ) -> Result<Response<Self::WatchRepositoriesStream>, Status> {
        let context = self.context(&request, "WatchRepositories")?;
        validate_optional_id(
            request.get_ref().repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let state = self.state.clone();
        let shutdown = self.shutdown.clone();
        let request_id = context.request_id.clone();
        let stream = repository_watch(state, shutdown, message, request_id);
        Ok(context.response(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl v1::runtime_profile_service_server::RuntimeProfileService for GrpcApi {
    async fn list_runtime_profiles(
        &self,
        request: Request<v1::ListRuntimeProfilesRequest>,
    ) -> Result<Response<v1::ListRuntimeProfilesResponse>, Status> {
        let context = self.context(&request, "ListRuntimeProfiles")?;
        let value = request.get_ref();
        validate_optional_id(
            value.repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            api_validation::enum_filter(
                value.kind,
                "kind",
                &[
                    v1::RuntimeProfileKind::AyniManaged as i32,
                    v1::RuntimeProfileKind::CustomRuntime as i32,
                ],
            )
        })
        .and_then(|()| {
            api_validation::enum_filter(
                value.state,
                "state",
                &[
                    v1::RuntimeProfileState::SetupRequired as i32,
                    v1::RuntimeProfileState::SetupPrOpen as i32,
                    v1::RuntimeProfileState::AwaitingEnvironment as i32,
                    v1::RuntimeProfileState::Ready as i32,
                    v1::RuntimeProfileState::Updating as i32,
                    v1::RuntimeProfileState::Degraded as i32,
                    v1::RuntimeProfileState::Failed as i32,
                ],
            )
        })
        .and_then(|()| api_validation::page(value.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_runtime_profiles(
                message.repository_connection_id.as_deref(),
                message.kind,
                message.state,
                message.page.as_ref(),
            )
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRuntimeProfilesResponse {
            runtime_profiles: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_runtime_profile(
        &self,
        request: Request<v1::GetRuntimeProfileRequest>,
    ) -> Result<Response<v1::GetRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "GetRuntimeProfile")?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.runtime_profile(&request.get_ref().runtime_profile_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRuntimeProfileResponse {
            runtime_profile: Some(resource),
        }))
    }

    async fn create_runtime_profile(
        &self,
        request: Request<v1::CreateRuntimeProfileRequest>,
    ) -> Result<Response<v1::CreateRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "CreateRuntimeProfile")?;
        api_validation::create_runtime_profile(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        if request
            .get_ref()
            .configuration
            .as_ref()
            .is_some_and(|configuration| {
                matches!(
                    configuration.configuration,
                    Some(v1::runtime_profile_configuration::Configuration::CustomRuntime(_))
                )
            })
        {
            return Err(context.unsupported_profile());
        }
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let (profile, revision) = with_store(&self.state, |store| {
            store.create_runtime_profile(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CreateRuntimeProfileResponse {
            runtime_profile: Some(profile),
            revision: Some(revision),
        }))
    }

    async fn list_runtime_profile_revisions(
        &self,
        request: Request<v1::ListRuntimeProfileRevisionsRequest>,
    ) -> Result<Response<v1::ListRuntimeProfileRevisionsResponse>, Status> {
        let context = self.context(&request, "ListRuntimeProfileRevisions")?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_runtime_profile_revisions(&message.runtime_profile_id, message.page.as_ref())
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRuntimeProfileRevisionsResponse {
            revisions: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_runtime_profile_revision(
        &self,
        request: Request<v1::GetRuntimeProfileRevisionRequest>,
    ) -> Result<Response<v1::GetRuntimeProfileRevisionResponse>, Status> {
        let context = self.context(&request, "GetRuntimeProfileRevision")?;
        validate_id(
            &request.get_ref().runtime_profile_revision_id,
            "runtime_profile_revision_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.runtime_profile_revision(&request.get_ref().runtime_profile_revision_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRuntimeProfileRevisionResponse {
            revision: Some(resource),
        }))
    }

    async fn list_runtime_artifacts(
        &self,
        request: Request<v1::ListRuntimeArtifactsRequest>,
    ) -> Result<Response<v1::ListRuntimeArtifactsResponse>, Status> {
        let context = self.context(&request, "ListRuntimeArtifacts")?;
        let value = request.get_ref();
        validate_id(&value.runtime_profile_id, "runtime_profile_id")
            .and_then(|()| {
                validate_optional_id(
                    value.runtime_profile_revision_id.as_deref(),
                    "runtime_profile_revision_id",
                )
            })
            .and_then(|()| {
                api_validation::enum_filter(
                    value.status,
                    "status",
                    &[
                        v1::RuntimeArtifactStatus::Discovered as i32,
                        v1::RuntimeArtifactStatus::Verifying as i32,
                        v1::RuntimeArtifactStatus::Verified as i32,
                        v1::RuntimeArtifactStatus::Rejected as i32,
                        v1::RuntimeArtifactStatus::Unavailable as i32,
                        v1::RuntimeArtifactStatus::Failed as i32,
                    ],
                )
            })
            .and_then(|()| api_validation::page(value.page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_runtime_artifacts(
                &message.runtime_profile_id,
                message.runtime_profile_revision_id.as_deref(),
                message.status,
                message.page.as_ref(),
            )
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListRuntimeArtifactsResponse {
            runtime_artifacts: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_runtime_artifact(
        &self,
        request: Request<v1::GetRuntimeArtifactRequest>,
    ) -> Result<Response<v1::GetRuntimeArtifactResponse>, Status> {
        let context = self.context(&request, "GetRuntimeArtifact")?;
        validate_id(
            &request.get_ref().runtime_artifact_id,
            "runtime_artifact_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.runtime_artifact(&request.get_ref().runtime_artifact_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetRuntimeArtifactResponse {
            runtime_artifact: Some(resource),
        }))
    }

    async fn list_automation_installations(
        &self,
        request: Request<v1::ListAutomationInstallationsRequest>,
    ) -> Result<Response<v1::ListAutomationInstallationsResponse>, Status> {
        let context = self.context(&request, "ListAutomationInstallations")?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_automation_installations(&message.runtime_profile_id, message.page.as_ref())
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListAutomationInstallationsResponse {
            automation_installations: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_automation_installation(
        &self,
        request: Request<v1::GetAutomationInstallationRequest>,
    ) -> Result<Response<v1::GetAutomationInstallationResponse>, Status> {
        let context = self.context(&request, "GetAutomationInstallation")?;
        validate_id(
            &request.get_ref().automation_installation_id,
            "automation_installation_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.automation_installation(&request.get_ref().automation_installation_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetAutomationInstallationResponse {
            automation_installation: Some(resource),
        }))
    }

    async fn install_repository_automation(
        &self,
        request: Request<v1::InstallRepositoryAutomationRequest>,
    ) -> Result<Response<v1::InstallRepositoryAutomationResponse>, Status> {
        let context = self.context(&request, "InstallRepositoryAutomation")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, |store| {
            store.install_repository_automation(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::InstallRepositoryAutomationResponse {
            automation_installation: Some(resource),
        }))
    }

    async fn reconcile_runtime_profile(
        &self,
        request: Request<v1::ReconcileRuntimeProfileRequest>,
    ) -> Result<Response<v1::ReconcileRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "ReconcileRuntimeProfile")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(&request.get_ref().runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let resource = with_store(&self.state, |store| {
            store.reconcile_runtime_profile(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ReconcileRuntimeProfileResponse {
            runtime_profile: Some(resource),
        }))
    }

    type WatchRuntimeProfilesStream = BoxGrpcStream<v1::WatchRuntimeProfilesResponse>;

    async fn watch_runtime_profiles(
        &self,
        request: Request<v1::WatchRuntimeProfilesRequest>,
    ) -> Result<Response<Self::WatchRuntimeProfilesStream>, Status> {
        let context = self.context(&request, "WatchRuntimeProfiles")?;
        validate_optional_id(
            request.get_ref().repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            validate_optional_id(
                request.get_ref().runtime_profile_id.as_deref(),
                "runtime_profile_id",
            )
        })
        .map_err(|error| context.validation(&error))?;
        let stream = profile_watch(
            self.state.clone(),
            self.shutdown.clone(),
            request.into_inner(),
            context.request_id.clone(),
        );
        Ok(context.response(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl v1::provider_service_server::ProviderService for GrpcApi {
    async fn list_provider_models(
        &self,
        request: Request<v1::ListProviderModelsRequest>,
    ) -> Result<Response<v1::ListProviderModelsResponse>, Status> {
        let context = self.context(&request, "ListProviderModels")?;
        api_validation::page(request.get_ref().page.as_ref())
            .map_err(|error| context.validation(&error))?;
        let catalog = self.state.model_catalog().ok_or_else(|| {
            context.status(
                Code::Unavailable,
                v1::ErrorReason::ProviderUnavailable,
                "the provider catalog is unavailable",
                None,
                None,
                None,
            )
        })?;
        let available = catalog.available().await.map_err(|_| {
            context.status(
                Code::Unavailable,
                v1::ErrorReason::ProviderUnavailable,
                "the provider catalog is unavailable",
                None,
                None,
                None,
            )
        })?;
        let size = request.get_ref().page.as_ref().map_or(50_usize, |page| {
            if page.page_size == 0 {
                50
            } else {
                usize::try_from(page.page_size).unwrap_or(100).min(100)
            }
        });
        let models = available
            .models
            .iter()
            .take(size)
            .map(|value| v1::ProviderModel {
                id: value.id.clone(),
                name: value.name.clone(),
                owned_by: value.owned_by.clone(),
                created_at: value
                    .created_at
                    .and_then(|seconds| i64::try_from(seconds).ok())
                    .map(|seconds| Timestamp { seconds, nanos: 0 }),
                context_length: value.context_length,
                max_output_tokens: value.max_output_tokens,
            })
            .collect();
        Ok(context.response(v1::ListProviderModelsResponse {
            provider: available.provider.to_owned(),
            models,
            page: Some(v1::PageResponse {
                next_page_token: String::new(),
                snapshot_cursor: 0,
            }),
            observed_at: i64::try_from(available.observed_at)
                .ok()
                .map(|seconds| Timestamp { seconds, nanos: 0 }),
        }))
    }
}

#[tonic::async_trait]
impl v1::development_task_service_server::DevelopmentTaskService for GrpcApi {
    async fn list_development_tasks(
        &self,
        request: Request<v1::ListDevelopmentTasksRequest>,
    ) -> Result<Response<v1::ListDevelopmentTasksResponse>, Status> {
        let context = self.context(&request, "ListDevelopmentTasks")?;
        let value = request.get_ref();
        validate_optional_id(
            value.repository_connection_id.as_deref(),
            "repository_connection_id",
        )
        .and_then(|()| {
            validate_optional_id(value.runtime_profile_id.as_deref(), "runtime_profile_id")
        })
        .and_then(|()| {
            api_validation::enum_filter(
                value.state,
                "state",
                &[
                    v1::DevelopmentTaskState::Queued as i32,
                    v1::DevelopmentTaskState::WaitingForRuntime as i32,
                    v1::DevelopmentTaskState::Running as i32,
                    v1::DevelopmentTaskState::Succeeded as i32,
                    v1::DevelopmentTaskState::Failed as i32,
                    v1::DevelopmentTaskState::Cancelled as i32,
                    v1::DevelopmentTaskState::DeadlineExceeded as i32,
                    v1::DevelopmentTaskState::Unsupported as i32,
                ],
            )
        })
        .and_then(|()| api_validation::page(value.page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_development_tasks(
                message.repository_connection_id.as_deref(),
                message.runtime_profile_id.as_deref(),
                message.state,
                message.page.as_ref(),
            )
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListDevelopmentTasksResponse {
            development_tasks: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_development_task(
        &self,
        request: Request<v1::GetDevelopmentTaskRequest>,
    ) -> Result<Response<v1::GetDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "GetDevelopmentTask")?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.development_task(&request.get_ref().development_task_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetDevelopmentTaskResponse {
            development_task: Some(resource),
        }))
    }

    async fn create_development_task(
        &self,
        request: Request<v1::CreateDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CreateDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CreateDevelopmentTask")?;
        api_validation::create_development_task(request.get_ref())
            .map_err(|error| context.validation(&error))?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        let message = request.into_inner();
        let (task, attempt) = with_store(&self.state, |store| {
            store.create_development_task(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CreateDevelopmentTaskResponse {
            development_task: Some(task),
            initial_attempt: Some(attempt),
        }))
    }

    async fn cancel_development_task(
        &self,
        request: Request<v1::CancelDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CancelDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CancelDevelopmentTask")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let task = with_store(&self.state, |store| {
            store.cancel_development_task(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::CancelDevelopmentTaskResponse {
            development_task: Some(task),
        }))
    }

    async fn retry_development_task(
        &self,
        request: Request<v1::RetryDevelopmentTaskRequest>,
    ) -> Result<Response<v1::RetryDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "RetryDevelopmentTask")?;
        let key = context.idempotency_key(&request.get_ref().idempotency_key)?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let attempt = with_store(&self.state, |store| {
            store.retry_development_task(&key, &message)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::RetryDevelopmentTaskResponse {
            attempt: Some(attempt),
        }))
    }

    async fn list_development_attempts(
        &self,
        request: Request<v1::ListDevelopmentAttemptsRequest>,
    ) -> Result<Response<v1::ListDevelopmentAttemptsResponse>, Status> {
        let context = self.context(&request, "ListDevelopmentAttempts")?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .and_then(|()| api_validation::page(request.get_ref().page.as_ref()))
        .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_development_attempts(&message.development_task_id, message.page.as_ref())
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListDevelopmentAttemptsResponse {
            attempts: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_development_attempt(
        &self,
        request: Request<v1::GetDevelopmentAttemptRequest>,
    ) -> Result<Response<v1::GetDevelopmentAttemptResponse>, Status> {
        let context = self.context(&request, "GetDevelopmentAttempt")?;
        validate_id(
            &request.get_ref().development_attempt_id,
            "development_attempt_id",
        )
        .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.development_attempt(&request.get_ref().development_attempt_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetDevelopmentAttemptResponse {
            attempt: Some(resource),
        }))
    }

    async fn list_candidates(
        &self,
        request: Request<v1::ListCandidatesRequest>,
    ) -> Result<Response<v1::ListCandidatesResponse>, Status> {
        let context = self.context(&request, "ListCandidates")?;
        let value = request.get_ref();
        validate_optional_id(value.development_task_id.as_deref(), "development_task_id")
            .and_then(|()| {
                validate_optional_id(
                    value.development_attempt_id.as_deref(),
                    "development_attempt_id",
                )
            })
            .and_then(|()| api_validation::page(value.page.as_ref()))
            .map_err(|error| context.validation(&error))?;
        let message = request.into_inner();
        let page = with_store(&self.state, |store| {
            store.list_candidates(
                message.development_task_id.as_deref(),
                message.development_attempt_id.as_deref(),
                message.page.as_ref(),
            )
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::ListCandidatesResponse {
            candidates: page.items.clone(),
            page: Some(page_response(&page)),
        }))
    }

    async fn get_candidate(
        &self,
        request: Request<v1::GetCandidateRequest>,
    ) -> Result<Response<v1::GetCandidateResponse>, Status> {
        let context = self.context(&request, "GetCandidate")?;
        validate_id(&request.get_ref().candidate_id, "candidate_id")
            .map_err(|error| context.validation(&error))?;
        let resource = with_store(&self.state, |store| {
            store.candidate(&request.get_ref().candidate_id)
        })
        .map_err(|error| context.store_error(&error))?;
        Ok(context.response(v1::GetCandidateResponse {
            candidate: Some(resource),
        }))
    }

    type WatchDevelopmentTaskStream = BoxGrpcStream<v1::WatchDevelopmentTaskResponse>;

    async fn watch_development_task(
        &self,
        request: Request<v1::WatchDevelopmentTaskRequest>,
    ) -> Result<Response<Self::WatchDevelopmentTaskStream>, Status> {
        let context = self.context(&request, "WatchDevelopmentTask")?;
        validate_id(
            &request.get_ref().development_task_id,
            "development_task_id",
        )
        .map_err(|error| context.validation(&error))?;
        with_store(&self.state, |store| {
            store
                .development_task(&request.get_ref().development_task_id)
                .map(|_| ())
        })
        .map_err(|error| context.store_error(&error))?;
        let stream = development_watch(
            self.state.clone(),
            self.shutdown.clone(),
            request.into_inner(),
            context.request_id.clone(),
        );
        Ok(context.response(Box::pin(stream)))
    }
}

include!("grpc_review.rs");

fn repository_watch(
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
    request: v1::WatchRepositoriesRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchRepositoriesResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(
                REPOSITORY_SERVICE, request.repository_connection_id.as_deref(), None)) {
                Ok((events, high_water)) => {
                    for event in events {
                        match repository_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let events = with_store(&state, |store| store.events_after(
                REPOSITORY_SERVICE, cursor, request.repository_connection_id.as_deref(), None));
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match repository_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {},
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
            }
        }
    }
}

fn profile_watch(
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
    request: v1::WatchRuntimeProfilesRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchRuntimeProfilesResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(
                PROFILE_SERVICE, request.repository_connection_id.as_deref(),
                request.runtime_profile_id.as_deref())) {
                Ok((events, high_water)) => {
                    for event in events {
                        match profile_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let events = with_store(&state, |store| store.events_after(
                PROFILE_SERVICE, cursor, request.repository_connection_id.as_deref(),
                request.runtime_profile_id.as_deref()));
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match profile_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
}

fn development_watch(
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
    request: v1::WatchDevelopmentTaskRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchDevelopmentTaskResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(
                DEVELOPMENT_SERVICE, None, Some(&request.development_task_id))) {
                Ok((events, high_water)) => {
                    for event in events {
                        match development_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let events = with_store(&state, |store| store.events_after(
                DEVELOPMENT_SERVICE, cursor, None, Some(&request.development_task_id)));
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match development_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
}

fn review_watch(
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
    request: v1::WatchReviewsRequest,
    request_id: String,
) -> impl Stream<Item = Result<v1::WatchReviewsResponse, Status>> + Send {
    async_stream::stream! {
        let mut cursor = request.after_cursor.unwrap_or(0);
        if request.after_cursor.is_none() {
            match with_store(&state, |store| store.initial_events(
                REVIEW_SERVICE, request.repository_connection_id.as_deref(),
                request.change_request_id.as_deref())) {
                Ok((events, high_water)) => {
                    for event in events {
                        match review_update(&event) {
                            Ok(response) => yield Ok(response),
                            Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                        }
                    }
                    cursor = high_water;
                }
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
        }
        loop {
            let events = with_store(&state, |store| store.events_after(
                REVIEW_SERVICE, cursor, request.repository_connection_id.as_deref(),
                request.change_request_id.as_deref()));
            match events {
                Ok(events) => for event in events {
                    cursor = event.cursor;
                    match review_update(&event) {
                        Ok(response) => yield Ok(response),
                        Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
                    }
                },
                Err(error) => { yield Err(stream_status(&error, &request_id)); return; }
            }
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {},
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return; }
            }
        }
    }
}

fn repository_update(
    event: &StoredEvent,
) -> Result<v1::WatchRepositoriesResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "repository_connection" => v1::watch_repositories_response::Update::RepositoryConnection(
            decode_event(&event.payload)?,
        ),
        "repository_snapshot" => v1::watch_repositories_response::Update::RepositorySnapshot(
            decode_event(&event.payload)?,
        ),
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchRepositoriesResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

fn profile_update(
    event: &StoredEvent,
) -> Result<v1::WatchRuntimeProfilesResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "runtime_profile" => v1::watch_runtime_profiles_response::Update::RuntimeProfile(
            decode_event(&event.payload)?,
        ),
        "runtime_profile_revision" => {
            v1::watch_runtime_profiles_response::Update::Revision(decode_event(&event.payload)?)
        }
        "runtime_artifact" => v1::watch_runtime_profiles_response::Update::RuntimeArtifact(
            decode_event(&event.payload)?,
        ),
        "automation_installation" => {
            v1::watch_runtime_profiles_response::Update::AutomationInstallation(decode_event(
                &event.payload,
            )?)
        }
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchRuntimeProfilesResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

fn development_update(
    event: &StoredEvent,
) -> Result<v1::WatchDevelopmentTaskResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "development_task" => v1::watch_development_task_response::Update::DevelopmentTask(
            decode_event(&event.payload)?,
        ),
        "development_attempt" => {
            v1::watch_development_task_response::Update::Attempt(decode_event(&event.payload)?)
        }
        "candidate" => {
            v1::watch_development_task_response::Update::Candidate(decode_event(&event.payload)?)
        }
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchDevelopmentTaskResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

fn review_update(event: &StoredEvent) -> Result<v1::WatchReviewsResponse, CleanStoreError> {
    let update = match event.resource_type.as_str() {
        "change_request" => {
            v1::watch_reviews_response::Update::ChangeRequest(decode_event(&event.payload)?)
        }
        "assessment" => {
            v1::watch_reviews_response::Update::Assessment(decode_event(&event.payload)?)
        }
        "evidence_reference" => {
            v1::watch_reviews_response::Update::EvidenceReference(decode_event(&event.payload)?)
        }
        _ => return Err(CleanStoreError::CorruptState),
    };
    Ok(v1::WatchReviewsResponse {
        cursor: event.cursor,
        update: Some(update),
    })
}

fn decode_event<M: Message + Default>(payload: &[u8]) -> Result<M, CleanStoreError> {
    M::decode(payload).map_err(|_| CleanStoreError::CorruptState)
}

fn stream_status(error: &CleanStoreError, request_id: &str) -> Status {
    let (code, reason) = if matches!(error, CleanStoreError::WatchCursorExpired) {
        (Code::OutOfRange, v1::ErrorReason::WatchCursorExpired)
    } else {
        (Code::Internal, v1::ErrorReason::Internal)
    };
    let message = crate::truncate_utf8(&error.to_string(), 512);
    let detail = v1::ErrorDetail {
        reason: reason as i32,
        message: message.clone(),
        resource_type: None,
        resource_id: None,
        metadata: Vec::new(),
        request_id: Some(request_id.to_owned()),
    };
    let rich = RichStatus {
        code: code as i32,
        message: message.clone(),
        details: vec![Any {
            type_url: ERROR_DETAIL_TYPE_URL.to_owned(),
            value: detail.encode_to_vec(),
        }],
    };
    Status::with_details(code, message, rich.encode_to_vec().into())
}

fn page_response<T>(page: &crate::clean_store::Page<T>) -> v1::PageResponse {
    v1::PageResponse {
        next_page_token: page.next_page_token.clone(),
        snapshot_cursor: page.snapshot_cursor,
    }
}
fn validate_id(value: &str, field: &'static str) -> Result<(), api_validation::ValidationError> {
    api_validation::resource_id(value, field)
}
fn validate_optional_id(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), api_validation::ValidationError> {
    value.map_or(Ok(()), |value| validate_id(value, field))
}
fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Clone, PartialEq, Message)]
struct RichStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

#[cfg(test)]
include!("grpc_tests.rs");
