//! Native gRPC adapters for the canonical clean-v1 product contract.
//!
//! The clean contract is served immediately, while resource persistence and
//! reconciliation are intentionally implemented by issue #55. Until then the
//! new resource operations validate their public request bounds and return
//! `UNIMPLEMENTED`; the independent provider-model catalog remains operational.

use crate::{
    AppState,
    application::{
        Application, ApplicationError, ErrorReason as ApplicationErrorReason,
        PageRequest as ApplicationPageRequest,
    },
    application_metrics::record_grpc_completion,
    grpc_access::{
        ClientIdentity, CustomClientAccess, CustomClientGuard,
        MAX_CONCURRENT_STREAMS_PER_CONNECTION, MAX_HTTP2_HEADER_LIST_BYTES,
    },
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

/// Product gRPC services backed only by transport-neutral application operations.
#[derive(Clone)]
pub struct GrpcApi {
    application: Application,
    custom_clients: Option<CustomClientGuard>,
}

impl GrpcApi {
    /// Creates adapters around shared control-plane state.
    #[must_use]
    pub(crate) fn new(state: AppState, _shutdown: watch::Receiver<bool>) -> Self {
        Self {
            application: Application::new(state),
            custom_clients: None,
        }
    }

    const fn new_custom(state: AppState, custom_clients: CustomClientGuard) -> Self {
        Self {
            application: Application::new(state),
            custom_clients: Some(custom_clients),
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

/// Serves the canonical API, standard health service, and development reflection.
///
/// # Errors
///
/// Returns a Tonic transport error when the listener cannot be started or served.
pub async fn serve(
    address: SocketAddr,
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), io::Error> {
    let api = GrpcApi::new(state, shutdown.clone());
    let repository = repository_server(api.clone());
    let profiles = profile_server(api.clone());
    let provider = provider_server(api.clone());
    let development = development_server(api.clone());
    let reviews = review_server(api);
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
        .add_service(repository)
        .add_service(profiles)
        .add_service(provider)
        .add_service(development)
        .add_service(reviews)
        .serve_with_shutdown(address, async move {
            if !*shutdown.borrow() {
                let _ = shutdown.changed().await;
            }
        })
        .await
        .map_err(io::Error::other)
}

/// Serves the product API on a TLS-only, authenticated custom-client listener.
///
/// Reflection and the standard health service remain deliberately absent.
///
/// # Errors
///
/// Returns an I/O error when TLS setup, listener startup, or serving fails.
pub async fn serve_custom(
    address: SocketAddr,
    state: AppState,
    mut shutdown: watch::Receiver<bool>,
    access: CustomClientAccess,
) -> Result<(), io::Error> {
    let api = GrpcApi::new_custom(state, access.guard());
    let repository = repository_server(api.clone());
    let profiles = profile_server(api.clone());
    let provider = provider_server(api.clone());
    let development = development_server(api.clone());
    let reviews = review_server(api);
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
        .add_service(repository)
        .add_service(profiles)
        .add_service(provider)
        .add_service(development)
        .add_service(reviews)
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
        let Some(client_id) = &self.client_id else {
            return Ok(key.to_owned());
        };
        let digest = Sha256::digest(
            format!(
                "ayni.custom-client.idempotency.v1\0{client_id}\0{}\0{key}",
                self.rpc
            )
            .as_bytes(),
        );
        Ok(format!("client-{digest:x}"))
    }

    fn validation(&self, error: &api_validation::ValidationError) -> Status {
        self.status(
            Code::InvalidArgument,
            error.reason,
            "the request violates the documented contract",
            Some(("field", error.field)),
        )
    }

    fn unsupported_profile(&self) -> Status {
        self.status(
            Code::FailedPrecondition,
            v1::ErrorReason::ProfileUnsupported,
            "Custom Runtime is modeled but is not operational in this release",
            None,
        )
    }

    fn unimplemented(&self) -> Status {
        self.status(
            Code::Unimplemented,
            v1::ErrorReason::NotImplemented,
            "clean-v1 resource persistence is not implemented yet",
            None,
        )
    }

    fn application_error(&self, error: &ApplicationError) -> Status {
        let code = grpc_code(error.reason);
        let reason = protobuf_error_reason(error.reason);
        let message = crate::truncate_utf8(&error.message, 512);
        self.status(code, reason, &message, None)
    }

    fn status(
        &self,
        code: Code,
        reason: v1::ErrorReason,
        message: &str,
        metadata: Option<(&str, &str)>,
    ) -> Status {
        record_grpc_completion(self.rpc, code);
        warn!(request_id = %self.request_id,
            client_id = self.client_id.as_deref().unwrap_or("trusted-local"), rpc = self.rpc,
            status = ?code, latency_ms = elapsed_millis(self.started), "gRPC request failed");
        let detail = v1::ErrorDetail {
            reason: reason as i32,
            message: message.to_owned(),
            resource_type: None,
            resource_id: None,
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

fn application_page(page: Option<v1::PageRequest>) -> ApplicationPageRequest {
    page.map_or_else(ApplicationPageRequest::default, |page| {
        ApplicationPageRequest {
            page_size: page.page_size,
            page_token: page.page_token,
        }
    })
}

const fn page_response(token: String) -> v1::PageResponse {
    v1::PageResponse {
        next_page_token: token,
        snapshot_cursor: 0,
    }
}

const fn validate_page(
    page: Option<&v1::PageRequest>,
) -> Result<(), api_validation::ValidationError> {
    api_validation::page(page)
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

macro_rules! stub_unary {
    ($name:ident, $request:ty, $response:ty, $rpc:literal, $validate:expr) => {
        fn $name<'life0, 'async_trait>(
            &'life0 self,
            request: Request<$request>,
        ) -> Pin<
            Box<
                dyn std::future::Future<Output = Result<Response<$response>, Status>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                let context = self.context(&request, $rpc)?;
                let message = request.into_inner();
                ($validate)(&message).map_err(|error| context.validation(&error))?;
                Err(context.unimplemented())
            })
        }
    };
}

#[tonic::async_trait]
impl v1::repository_service_server::RepositoryService for GrpcApi {
    stub_unary!(
        list_repository_connections,
        v1::ListRepositoryConnectionsRequest,
        v1::ListRepositoryConnectionsResponse,
        "ListRepositoryConnections",
        |value: &v1::ListRepositoryConnectionsRequest| {
            api_validation::enum_filter(
                value.status,
                "status",
                &[
                    v1::RepositoryConnectionStatus::Connecting as i32,
                    v1::RepositoryConnectionStatus::Connected as i32,
                    v1::RepositoryConnectionStatus::Degraded as i32,
                    v1::RepositoryConnectionStatus::Revoked as i32,
                    v1::RepositoryConnectionStatus::Failed as i32,
                ],
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_repository_connection,
        v1::GetRepositoryConnectionRequest,
        v1::GetRepositoryConnectionResponse,
        "GetRepositoryConnection",
        |value: &v1::GetRepositoryConnectionRequest| validate_id(
            &value.repository_connection_id,
            "repository_connection_id"
        )
    );

    async fn create_repository_connection(
        &self,
        request: Request<v1::CreateRepositoryConnectionRequest>,
    ) -> Result<Response<v1::CreateRepositoryConnectionResponse>, Status> {
        let context = self.context(&request, "CreateRepositoryConnection")?;
        let message = request.into_inner();
        api_validation::create_repository_connection(&message)
            .map_err(|error| context.validation(&error))?;
        let _ = context.idempotency_key(&message.idempotency_key)?;
        Err(context.unimplemented())
    }

    stub_unary!(
        list_repository_snapshots,
        v1::ListRepositorySnapshotsRequest,
        v1::ListRepositorySnapshotsResponse,
        "ListRepositorySnapshots",
        |value: &v1::ListRepositorySnapshotsRequest| {
            validate_id(&value.repository_connection_id, "repository_connection_id")?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_repository_snapshot,
        v1::GetRepositorySnapshotRequest,
        v1::GetRepositorySnapshotResponse,
        "GetRepositorySnapshot",
        |value: &v1::GetRepositorySnapshotRequest| validate_id(
            &value.repository_snapshot_id,
            "repository_snapshot_id"
        )
    );

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
        Err(context.unimplemented())
    }
}

#[tonic::async_trait]
impl v1::runtime_profile_service_server::RuntimeProfileService for GrpcApi {
    stub_unary!(
        list_runtime_profiles,
        v1::ListRuntimeProfilesRequest,
        v1::ListRuntimeProfilesResponse,
        "ListRuntimeProfiles",
        |value: &v1::ListRuntimeProfilesRequest| {
            validate_optional_id(
                value.repository_connection_id.as_deref(),
                "repository_connection_id",
            )?;
            api_validation::enum_filter(
                value.kind,
                "kind",
                &[
                    v1::RuntimeProfileKind::AyniManaged as i32,
                    v1::RuntimeProfileKind::CustomRuntime as i32,
                ],
            )?;
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
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_runtime_profile,
        v1::GetRuntimeProfileRequest,
        v1::GetRuntimeProfileResponse,
        "GetRuntimeProfile",
        |value: &v1::GetRuntimeProfileRequest| validate_id(
            &value.runtime_profile_id,
            "runtime_profile_id"
        )
    );

    async fn create_runtime_profile(
        &self,
        request: Request<v1::CreateRuntimeProfileRequest>,
    ) -> Result<Response<v1::CreateRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "CreateRuntimeProfile")?;
        let message = request.into_inner();
        api_validation::create_runtime_profile(&message)
            .map_err(|error| context.validation(&error))?;
        let _ = context.idempotency_key(&message.idempotency_key)?;
        if message.configuration.as_ref().is_some_and(|configuration| {
            matches!(
                configuration.configuration,
                Some(v1::runtime_profile_configuration::Configuration::CustomRuntime(_))
            )
        }) {
            return Err(context.unsupported_profile());
        }
        Err(context.unimplemented())
    }

    stub_unary!(
        list_runtime_profile_revisions,
        v1::ListRuntimeProfileRevisionsRequest,
        v1::ListRuntimeProfileRevisionsResponse,
        "ListRuntimeProfileRevisions",
        |value: &v1::ListRuntimeProfileRevisionsRequest| {
            validate_id(&value.runtime_profile_id, "runtime_profile_id")?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_runtime_profile_revision,
        v1::GetRuntimeProfileRevisionRequest,
        v1::GetRuntimeProfileRevisionResponse,
        "GetRuntimeProfileRevision",
        |value: &v1::GetRuntimeProfileRevisionRequest| validate_id(
            &value.runtime_profile_revision_id,
            "runtime_profile_revision_id"
        )
    );
    stub_unary!(
        list_runtime_artifacts,
        v1::ListRuntimeArtifactsRequest,
        v1::ListRuntimeArtifactsResponse,
        "ListRuntimeArtifacts",
        |value: &v1::ListRuntimeArtifactsRequest| {
            validate_id(&value.runtime_profile_id, "runtime_profile_id")?;
            validate_optional_id(
                value.runtime_profile_revision_id.as_deref(),
                "runtime_profile_revision_id",
            )?;
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
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_runtime_artifact,
        v1::GetRuntimeArtifactRequest,
        v1::GetRuntimeArtifactResponse,
        "GetRuntimeArtifact",
        |value: &v1::GetRuntimeArtifactRequest| validate_id(
            &value.runtime_artifact_id,
            "runtime_artifact_id"
        )
    );
    stub_unary!(
        list_automation_installations,
        v1::ListAutomationInstallationsRequest,
        v1::ListAutomationInstallationsResponse,
        "ListAutomationInstallations",
        |value: &v1::ListAutomationInstallationsRequest| {
            validate_id(&value.runtime_profile_id, "runtime_profile_id")?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_automation_installation,
        v1::GetAutomationInstallationRequest,
        v1::GetAutomationInstallationResponse,
        "GetAutomationInstallation",
        |value: &v1::GetAutomationInstallationRequest| validate_id(
            &value.automation_installation_id,
            "automation_installation_id"
        )
    );

    async fn install_repository_automation(
        &self,
        request: Request<v1::InstallRepositoryAutomationRequest>,
    ) -> Result<Response<v1::InstallRepositoryAutomationResponse>, Status> {
        let context = self.context(&request, "InstallRepositoryAutomation")?;
        let message = request.into_inner();
        let _ = context.idempotency_key(&message.idempotency_key)?;
        validate_id(&message.runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        Err(context.unimplemented())
    }

    async fn reconcile_runtime_profile(
        &self,
        request: Request<v1::ReconcileRuntimeProfileRequest>,
    ) -> Result<Response<v1::ReconcileRuntimeProfileResponse>, Status> {
        let context = self.context(&request, "ReconcileRuntimeProfile")?;
        let message = request.into_inner();
        let _ = context.idempotency_key(&message.idempotency_key)?;
        validate_id(&message.runtime_profile_id, "runtime_profile_id")
            .map_err(|error| context.validation(&error))?;
        Err(context.unimplemented())
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
        Err(context.unimplemented())
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
        let request = request.into_inner();
        let (available, page) = self
            .application
            .list_provider_models(&application_page(request.page))
            .await
            .map_err(|error| context.application_error(&error))?;
        Ok(context.response(v1::ListProviderModelsResponse {
            provider: available.provider.to_owned(),
            models: page
                .items
                .into_iter()
                .map(|value| v1::ProviderModel {
                    id: value.id,
                    name: value.name,
                    owned_by: value.owned_by,
                    created_at: value.created_at.and_then(|seconds| {
                        i64::try_from(seconds)
                            .ok()
                            .map(|seconds| Timestamp { seconds, nanos: 0 })
                    }),
                    context_length: value.context_length,
                    max_output_tokens: value.max_output_tokens,
                })
                .collect(),
            page: Some(page_response(page.next_page_token)),
            observed_at: i64::try_from(available.observed_at)
                .ok()
                .map(|seconds| Timestamp { seconds, nanos: 0 }),
        }))
    }
}

#[tonic::async_trait]
impl v1::development_task_service_server::DevelopmentTaskService for GrpcApi {
    stub_unary!(
        list_development_tasks,
        v1::ListDevelopmentTasksRequest,
        v1::ListDevelopmentTasksResponse,
        "ListDevelopmentTasks",
        |value: &v1::ListDevelopmentTasksRequest| {
            validate_optional_id(
                value.repository_connection_id.as_deref(),
                "repository_connection_id",
            )?;
            validate_optional_id(value.runtime_profile_id.as_deref(), "runtime_profile_id")?;
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
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_development_task,
        v1::GetDevelopmentTaskRequest,
        v1::GetDevelopmentTaskResponse,
        "GetDevelopmentTask",
        |value: &v1::GetDevelopmentTaskRequest| validate_id(
            &value.development_task_id,
            "development_task_id"
        )
    );

    async fn create_development_task(
        &self,
        request: Request<v1::CreateDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CreateDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CreateDevelopmentTask")?;
        let message = request.into_inner();
        api_validation::create_development_task(&message)
            .map_err(|error| context.validation(&error))?;
        let _ = context.idempotency_key(&message.idempotency_key)?;
        Err(context.unimplemented())
    }

    async fn cancel_development_task(
        &self,
        request: Request<v1::CancelDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CancelDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CancelDevelopmentTask")?;
        let message = request.into_inner();
        let _ = context.idempotency_key(&message.idempotency_key)?;
        validate_id(&message.development_task_id, "development_task_id")
            .map_err(|error| context.validation(&error))?;
        Err(context.unimplemented())
    }

    async fn retry_development_task(
        &self,
        request: Request<v1::RetryDevelopmentTaskRequest>,
    ) -> Result<Response<v1::RetryDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "RetryDevelopmentTask")?;
        let message = request.into_inner();
        let _ = context.idempotency_key(&message.idempotency_key)?;
        validate_id(&message.development_task_id, "development_task_id")
            .map_err(|error| context.validation(&error))?;
        Err(context.unimplemented())
    }

    stub_unary!(
        list_development_attempts,
        v1::ListDevelopmentAttemptsRequest,
        v1::ListDevelopmentAttemptsResponse,
        "ListDevelopmentAttempts",
        |value: &v1::ListDevelopmentAttemptsRequest| {
            validate_id(&value.development_task_id, "development_task_id")?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_development_attempt,
        v1::GetDevelopmentAttemptRequest,
        v1::GetDevelopmentAttemptResponse,
        "GetDevelopmentAttempt",
        |value: &v1::GetDevelopmentAttemptRequest| validate_id(
            &value.development_attempt_id,
            "development_attempt_id"
        )
    );
    stub_unary!(
        list_candidates,
        v1::ListCandidatesRequest,
        v1::ListCandidatesResponse,
        "ListCandidates",
        |value: &v1::ListCandidatesRequest| {
            validate_optional_id(value.development_task_id.as_deref(), "development_task_id")?;
            validate_optional_id(
                value.development_attempt_id.as_deref(),
                "development_attempt_id",
            )?;
            validate_page(value.page.as_ref())
        }
    );
    stub_unary!(
        get_candidate,
        v1::GetCandidateRequest,
        v1::GetCandidateResponse,
        "GetCandidate",
        |value: &v1::GetCandidateRequest| validate_id(&value.candidate_id, "candidate_id")
    );

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
        Err(context.unimplemented())
    }
}

include!("grpc_review.rs");

#[derive(Clone, PartialEq, Message)]
struct RichStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

const fn grpc_code(reason: ApplicationErrorReason) -> Code {
    match reason {
        ApplicationErrorReason::InvalidIdentifier
        | ApplicationErrorReason::InvalidInput
        | ApplicationErrorReason::InvalidSourceRevision
        | ApplicationErrorReason::InvalidLockDigest => Code::InvalidArgument,
        ApplicationErrorReason::IdempotencyKeyReused => Code::AlreadyExists,
        ApplicationErrorReason::ResourceNotFound | ApplicationErrorReason::EvidenceUnavailable => {
            Code::NotFound
        }
        ApplicationErrorReason::Conflict => Code::Aborted,
        ApplicationErrorReason::FailedPrecondition
        | ApplicationErrorReason::TaskNotActionable
        | ApplicationErrorReason::ModelUnavailable
        | ApplicationErrorReason::RepositoryNotReady
        | ApplicationErrorReason::TaskPhaseNotEligible
        | ApplicationErrorReason::TaskDeadlineExpired => Code::FailedPrecondition,
        ApplicationErrorReason::ProviderUnavailable
        | ApplicationErrorReason::RepositoryAccess
        | ApplicationErrorReason::DependencyUnavailable => Code::Unavailable,
        ApplicationErrorReason::Cancelled => Code::Cancelled,
        ApplicationErrorReason::DeadlineExceeded => Code::DeadlineExceeded,
        ApplicationErrorReason::Internal => Code::Internal,
    }
}

const fn protobuf_error_reason(reason: ApplicationErrorReason) -> v1::ErrorReason {
    match reason {
        ApplicationErrorReason::InvalidIdentifier => v1::ErrorReason::InvalidIdentifier,
        ApplicationErrorReason::InvalidInput => v1::ErrorReason::InvalidInput,
        ApplicationErrorReason::IdempotencyKeyReused => v1::ErrorReason::IdempotencyKeyReused,
        ApplicationErrorReason::ResourceNotFound => v1::ErrorReason::ResourceNotFound,
        ApplicationErrorReason::Conflict => v1::ErrorReason::Conflict,
        ApplicationErrorReason::FailedPrecondition => v1::ErrorReason::FailedPrecondition,
        ApplicationErrorReason::TaskNotActionable
        | ApplicationErrorReason::TaskPhaseNotEligible => v1::ErrorReason::TaskNotActionable,
        ApplicationErrorReason::ProviderUnavailable => v1::ErrorReason::ProviderUnavailable,
        ApplicationErrorReason::ModelUnavailable => v1::ErrorReason::ModelUnavailable,
        ApplicationErrorReason::RepositoryAccess => v1::ErrorReason::RepositoryAccess,
        ApplicationErrorReason::EvidenceUnavailable => v1::ErrorReason::EvidenceUnavailable,
        ApplicationErrorReason::DependencyUnavailable | ApplicationErrorReason::Internal => {
            v1::ErrorReason::Internal
        }
        ApplicationErrorReason::Cancelled => v1::ErrorReason::Cancelled,
        ApplicationErrorReason::DeadlineExceeded => v1::ErrorReason::DeadlineExceeded,
        ApplicationErrorReason::InvalidSourceRevision => v1::ErrorReason::InvalidRepositoryRevision,
        ApplicationErrorReason::InvalidLockDigest => v1::ErrorReason::InvalidLockFingerprint,
        ApplicationErrorReason::RepositoryNotReady => v1::ErrorReason::ProfileNotReady,
        ApplicationErrorReason::TaskDeadlineExpired => v1::ErrorReason::TaskDeadlineExpired,
    }
}

#[cfg(test)]
mod tests {
    include!("grpc_tests.rs");
}
