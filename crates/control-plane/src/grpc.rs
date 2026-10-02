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
            CleanStoreError::ParentMismatch
            | CleanStoreError::InvalidTransition
            | CleanStoreError::UnsupportedWorkflowTemplate(_) => (
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

mod providers_tasks;
mod repositories_profiles;
#[path = "grpc/watch.rs"]
mod watch_streams;

use watch_streams::*;

include!("grpc_review.rs");

#[cfg(test)]
include!("grpc_tests.rs");
