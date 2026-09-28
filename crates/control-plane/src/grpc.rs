//! Native gRPC adapters for the canonical v1 product contract.

use crate::{
    AppState,
    application::{
        Application, ApplicationError, CreateDevelopmentTaskCommand, DevelopmentTaskListItem,
        ErrorReason as ApplicationErrorReason, PageRequest as ApplicationPageRequest,
    },
    application_metrics::record_grpc_completion,
    development::{
        DevelopmentLifecycleOutcome, DevelopmentLifecycleStage, DevelopmentPhase, DevelopmentTask,
        RepositoryBinding,
    },
    grpc_access::{
        ClientIdentity, CustomClientAccess, CustomClientGuard,
        MAX_CONCURRENT_STREAMS_PER_CONNECTION, MAX_HTTP2_HEADER_LIST_BYTES,
    },
    model_usage::{
        EstimatedModelCost, ModelUsageCollectionStatus, ModelUsageInteraction, ModelUsageSummary,
        StageModelUsage, TaskModelUsage,
    },
    preparation::{PreparationStatus, PreparationView, RepositoryView},
    publication::EvidenceView,
};
use ayni_orchestrator_api::{FILE_DESCRIPTOR_SET, v1};
use ayni_orchestrator_domain::{
    ChangeConstraints, ModelInteractionOutcome, ModelUsageStage, ProviderTokenUsage,
};
use prost::Message;
use prost_types::{Any, Duration, Timestamp};
use sha2::{Digest as _, Sha256};
use std::{io, net::SocketAddr, pin::Pin, str::FromStr, time::Instant};
use tokio::sync::watch;
use tokio_stream::{Stream, StreamExt as _};
use tonic::{
    Code, Request, Response, Status,
    metadata::MetadataValue,
    transport::{Identity, Server, ServerTlsConfig},
};
use tracing::{info, warn};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail";
type BoxGrpcStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

/// Product gRPC services backed only by transport-neutral application operations.
#[derive(Clone)]
pub struct GrpcApi {
    application: Application,
    shutdown: watch::Receiver<bool>,
    custom_clients: Option<CustomClientGuard>,
}

impl GrpcApi {
    /// Creates adapters around shared control-plane state and process shutdown.
    #[must_use]
    pub(crate) const fn new(state: AppState, shutdown: watch::Receiver<bool>) -> Self {
        Self {
            application: Application::new(state),
            shutdown,
            custom_clients: None,
        }
    }

    const fn new_custom(
        state: AppState,
        shutdown: watch::Receiver<bool>,
        custom_clients: CustomClientGuard,
    ) -> Self {
        Self {
            application: Application::new(state),
            shutdown,
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
    let repository = v1::repository_service_server::RepositoryServiceServer::new(api.clone())
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES);
    let provider = v1::provider_service_server::ProviderServiceServer::new(api.clone())
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES);
    let development = v1::development_task_service_server::DevelopmentTaskServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES);
    let (health_reporter, health) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<v1::repository_service_server::RepositoryServiceServer<GrpcApi>>()
        .await;
    health_reporter
        .set_serving::<v1::provider_service_server::ProviderServiceServer<GrpcApi>>()
        .await;
    health_reporter
        .set_serving::<v1::development_task_service_server::DevelopmentTaskServiceServer<GrpcApi>>()
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
        .add_service(provider)
        .add_service(development)
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
/// Reflection is deliberately omitted: approved clients receive the canonical
/// descriptor bundle through the private contract distribution channel.
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
    let api = GrpcApi::new_custom(state, shutdown.clone(), access.guard());
    let repository = v1::repository_service_server::RepositoryServiceServer::new(api.clone())
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES);
    let provider = v1::provider_service_server::ProviderServiceServer::new(api.clone())
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES);
    let development = v1::development_task_service_server::DevelopmentTaskServiceServer::new(api)
        .max_decoding_message_size(MAX_REQUEST_BYTES)
        .max_encoding_message_size(MAX_RESPONSE_BYTES);
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
        .add_service(provider)
        .add_service(development)
        .serve_with_shutdown(address, async move {
            if !*shutdown.borrow() {
                let _ = shutdown.changed().await;
            }
        })
        .await
        .map_err(io::Error::other)
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
        let client_id = client.map(|identity| identity.id().to_owned());
        Self {
            request_id,
            client_id,
            rpc,
            started: Instant::now(),
        }
    }

    #[allow(clippy::unnecessary_wraps)] // Generated service methods return Result; this centralizes metadata.
    fn ok<T>(&self, value: T) -> Result<Response<T>, Status> {
        record_grpc_completion(self.rpc, Code::Ok);
        let mut response = Response::new(value);
        if let Ok(value) = MetadataValue::from_str(&self.request_id) {
            response.metadata_mut().insert("x-request-id", value);
        }
        info!(
            request_id = %self.request_id,
            client_id = self.client_id.as_deref().unwrap_or("trusted-local"),
            rpc = self.rpc,
            status = "ok",
            latency_ms = elapsed_millis(self.started),
            "gRPC request completed"
        );
        Ok(response)
    }

    fn stream<T, S>(self, stream: S) -> Response<BoxGrpcStream<T>>
    where
        T: Send + 'static,
        S: Stream<Item = Result<T, Status>> + Send + 'static,
    {
        let mut response = Response::new({
            let mut completion = StreamCompletion::new(&self);
            let audited = async_stream::stream! {
                tokio::pin!(stream);
                while let Some(item) = stream.next().await {
                    if let Err(status) = &item {
                        completion.finish(status.code());
                        yield item;
                        return;
                    }
                    yield item;
                }
                completion.finish(Code::Ok);
            };
            Box::pin(audited) as BoxGrpcStream<T>
        });
        if let Ok(value) = MetadataValue::from_str(&self.request_id) {
            response.metadata_mut().insert("x-request-id", value);
        }
        response
    }

    fn error(&self, error: ApplicationError) -> Status {
        let status = application_status(error, Some(&self.request_id));
        record_grpc_completion(self.rpc, status.code());
        warn!(
            request_id = %self.request_id,
            client_id = self.client_id.as_deref().unwrap_or("trusted-local"),
            rpc = self.rpc,
            status = ?status.code(),
            latency_ms = elapsed_millis(self.started),
            "gRPC request failed"
        );
        status
    }

    fn idempotency_key(&self, key: String) -> Result<String, Status> {
        let Some(client_id) = &self.client_id else {
            return Ok(key);
        };
        if key.is_empty()
            || key.len() > 128
            || !key.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
        {
            return Err(self.invalid("idempotency_key must be 1-128 safe ASCII bytes"));
        }
        let digest = Sha256::digest(
            format!(
                "ayni.custom-client.idempotency.v1\0{client_id}\0{}\0{key}",
                self.rpc
            )
            .as_bytes(),
        );
        Ok(format!("client-{digest:x}"))
    }

    fn invalid(&self, message: &'static str) -> Status {
        self.error(ApplicationError {
            reason: ApplicationErrorReason::InvalidInput,
            message: message.to_owned(),
            resource_type: None,
            resource_id: None,
        })
    }
}

struct StreamCompletion {
    request_id: String,
    client_id: Option<String>,
    rpc: &'static str,
    started: Instant,
    finished: bool,
}

impl StreamCompletion {
    fn new(context: &RequestContext) -> Self {
        Self {
            request_id: context.request_id.clone(),
            client_id: context.client_id.clone(),
            rpc: context.rpc,
            started: context.started,
            finished: false,
        }
    }

    fn finish(&mut self, code: Code) {
        if self.finished {
            return;
        }
        self.finished = true;
        record_grpc_completion(self.rpc, code);
        if code == Code::Ok {
            info!(
                request_id = %self.request_id,
                client_id = self.client_id.as_deref().unwrap_or("trusted-local"),
                rpc = self.rpc,
                status = "ok",
                latency_ms = elapsed_millis(self.started),
                "gRPC stream completed"
            );
        } else {
            warn!(
                request_id = %self.request_id,
                client_id = self.client_id.as_deref().unwrap_or("trusted-local"),
                rpc = self.rpc,
                status = ?code,
                latency_ms = elapsed_millis(self.started),
                "gRPC stream ended"
            );
        }
    }
}

impl Drop for StreamCompletion {
    fn drop(&mut self) {
        self.finish(Code::Cancelled);
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

fn request_deadline<T>(request: &Request<T>) -> Option<tokio::time::Instant> {
    let value = request.metadata().get("grpc-timeout")?.to_str().ok()?;
    if value.len() < 2 || value.len() > 9 {
        return None;
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    let amount = digits.parse::<u64>().ok()?;
    let duration = match unit {
        "H" => std::time::Duration::from_secs(amount.checked_mul(3_600)?),
        "M" => std::time::Duration::from_secs(amount.checked_mul(60)?),
        "S" => std::time::Duration::from_secs(amount),
        "m" => std::time::Duration::from_millis(amount),
        "u" => std::time::Duration::from_micros(amount),
        "n" => std::time::Duration::from_nanos(amount),
        _ => return None,
    };
    Some(tokio::time::Instant::now() + duration)
}

fn watch_deadline_status(task_id: &str, request_id: &str) -> Status {
    application_status(
        ApplicationError {
            reason: ApplicationErrorReason::DeadlineExceeded,
            message: "watch request deadline exceeded".to_owned(),
            resource_type: Some("development_task"),
            resource_id: Some(task_id.to_owned()),
        },
        Some(request_id),
    )
}

fn application_page(page: Option<v1::PageRequest>) -> ApplicationPageRequest {
    page.map_or_else(ApplicationPageRequest::default, |page| {
        ApplicationPageRequest {
            page_size: page.page_size,
            page_token: page.page_token,
        }
    })
}

#[allow(clippy::unnecessary_wraps)] // Prost represents message fields as Option.
const fn page_response(token: String) -> Option<v1::PageResponse> {
    Some(v1::PageResponse {
        next_page_token: token,
    })
}

#[tonic::async_trait]
impl v1::repository_service_server::RepositoryService for GrpcApi {
    async fn list_repositories(
        &self,
        request: Request<v1::ListRepositoriesRequest>,
    ) -> Result<Response<v1::ListRepositoriesResponse>, Status> {
        let context = self.context(&request, "ListRepositories")?;
        let request = request.into_inner();
        let page = self
            .application
            .list_repositories(&application_page(request.page))
            .map_err(|error| context.error(error))?;
        context.ok(v1::ListRepositoriesResponse {
            repositories: page.items.into_iter().map(repository_binding).collect(),
            page: page_response(page.next_page_token),
        })
    }

    async fn get_repository(
        &self,
        request: Request<v1::GetRepositoryRequest>,
    ) -> Result<Response<v1::GetRepositoryResponse>, Status> {
        let context = self.context(&request, "GetRepository")?;
        let request = request.into_inner();
        let repository = self
            .application
            .get_repository(&request.repository_id)
            .map_err(|error| context.error(error))?;
        context.ok(v1::GetRepositoryResponse {
            repository: Some(repository_binding(repository)),
        })
    }

    async fn save_repository(
        &self,
        request: Request<v1::SaveRepositoryRequest>,
    ) -> Result<Response<v1::SaveRepositoryResponse>, Status> {
        let context = self.context(&request, "SaveRepository")?;
        let request = request.into_inner();
        let repository = request
            .repository
            .ok_or_else(|| context.invalid("repository is required"))?;
        let installation_id = i64::try_from(repository.installation_id)
            .map_err(|_| context.invalid("installation_id exceeds the supported range"))?;
        let binding = RepositoryBinding {
            id: repository.id,
            owner: repository.owner,
            repository: repository.repository,
            installation_id,
            base_branch: repository.base_branch,
            lock_sha256: repository.lock_sha256,
        };
        let repository = self
            .application
            .save_repository(context.idempotency_key(request.idempotency_key)?, &binding)
            .map_err(|error| context.error(error))?;
        context.ok(v1::SaveRepositoryResponse {
            repository: Some(repository_binding(repository)),
        })
    }

    async fn list_preparations(
        &self,
        request: Request<v1::ListPreparationsRequest>,
    ) -> Result<Response<v1::ListPreparationsResponse>, Status> {
        let context = self.context(&request, "ListPreparations")?;
        let request = request.into_inner();
        let page = self
            .application
            .list_preparations(&request.repository_id, &application_page(request.page))
            .map_err(|error| context.error(error))?;
        context.ok(v1::ListPreparationsResponse {
            preparations: page.items.into_iter().map(preparation_message).collect(),
            page: page_response(page.next_page_token),
        })
    }

    async fn get_preparation(
        &self,
        request: Request<v1::GetPreparationRequest>,
    ) -> Result<Response<v1::GetPreparationResponse>, Status> {
        let context = self.context(&request, "GetPreparation")?;
        let request = request.into_inner();
        let preparation = self
            .application
            .get_preparation(&request.preparation_id)
            .map_err(|error| context.error(error))?;
        context.ok(v1::GetPreparationResponse {
            preparation: Some(preparation_message(preparation)),
        })
    }

    async fn prepare_repository(
        &self,
        request: Request<v1::PrepareRepositoryRequest>,
    ) -> Result<Response<v1::PrepareRepositoryResponse>, Status> {
        let context = self.context(&request, "PrepareRepository")?;
        let request = request.into_inner();
        let preparation = self
            .application
            .prepare_repository(
                context.idempotency_key(request.idempotency_key)?,
                &request.repository_id,
                request.revision,
            )
            .map_err(|error| context.error(error))?;
        let reused = preparation.reused;
        context.ok(v1::PrepareRepositoryResponse {
            preparation: Some(preparation_message(preparation)),
            reused,
        })
    }
}

#[tonic::async_trait]
impl v1::provider_service_server::ProviderService for GrpcApi {
    async fn list_provider_models(
        &self,
        request: Request<v1::ListProviderModelsRequest>,
    ) -> Result<Response<v1::ListProviderModelsResponse>, Status> {
        let context = self.context(&request, "ListProviderModels")?;
        let request = request.into_inner();
        let (available, page) = self
            .application
            .list_provider_models(&application_page(request.page))
            .await
            .map_err(|error| context.error(error))?;
        context.ok(v1::ListProviderModelsResponse {
            provider: available.provider.to_owned(),
            models: page.items.into_iter().map(provider_model).collect(),
            page: page_response(page.next_page_token),
            observed_at: Some(Timestamp {
                seconds: i64::try_from(available.observed_at)
                    .map_err(|_| context.invalid("provider observation time is out of range"))?,
                nanos: 0,
            }),
        })
    }
}

#[tonic::async_trait]
impl v1::development_task_service_server::DevelopmentTaskService for GrpcApi {
    async fn list_development_tasks(
        &self,
        request: Request<v1::ListDevelopmentTasksRequest>,
    ) -> Result<Response<v1::ListDevelopmentTasksResponse>, Status> {
        let context = self.context(&request, "ListDevelopmentTasks")?;
        let request = request.into_inner();
        let phase = request
            .phase
            .map(|value| development_phase_from_proto(value, &context))
            .transpose()?;
        let page = self
            .application
            .list_development_tasks(phase, &application_page(request.page))
            .map_err(|error| context.error(error))?;
        context.ok(v1::ListDevelopmentTasksResponse {
            tasks: page.items.into_iter().map(development_summary).collect(),
            page: page_response(page.next_page_token),
        })
    }

    async fn get_development_task(
        &self,
        request: Request<v1::GetDevelopmentTaskRequest>,
    ) -> Result<Response<v1::GetDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "GetDevelopmentTask")?;
        let request = request.into_inner();
        let task = self
            .application
            .get_development_task(&request.task_id)
            .map_err(|error| context.error(error))?;
        context.ok(v1::GetDevelopmentTaskResponse {
            task: Some(development_task(task)),
        })
    }

    async fn create_development_task(
        &self,
        request: Request<v1::CreateDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CreateDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CreateDevelopmentTask")?;
        let request = request.into_inner();
        let input = request
            .input
            .ok_or_else(|| context.invalid("input is required"))?;
        let deadline_seconds = input
            .deadline
            .map(|duration| duration_seconds(duration, &context))
            .transpose()?;
        let constraints =
            input
                .change_constraints
                .map_or_else(ChangeConstraints::default, |constraints| {
                    ChangeConstraints {
                        protected_paths: constraints.protected_paths,
                        allow_test_edits: constraints.allow_test_edits,
                    }
                });
        let task = self
            .application
            .create_development_task(CreateDevelopmentTaskCommand {
                idempotency_key: context.idempotency_key(request.idempotency_key)?,
                repository_id: input.repository_id,
                model: input.model,
                start_revision: input.start_revision,
                request: input.request,
                acceptance_criteria: input.acceptance_criteria,
                change_constraints: constraints,
                deadline_seconds,
            })
            .await
            .map_err(|error| context.error(error))?;
        context.ok(v1::CreateDevelopmentTaskResponse {
            task: Some(development_task(task)),
        })
    }

    async fn cancel_development_task(
        &self,
        request: Request<v1::CancelDevelopmentTaskRequest>,
    ) -> Result<Response<v1::CancelDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "CancelDevelopmentTask")?;
        let request = request.into_inner();
        let task = self
            .application
            .cancel_development_task(
                context.idempotency_key(request.idempotency_key)?,
                &request.task_id,
            )
            .map_err(|error| context.error(error))?;
        context.ok(v1::CancelDevelopmentTaskResponse {
            task: Some(development_task(task)),
        })
    }

    async fn retry_development_task(
        &self,
        request: Request<v1::RetryDevelopmentTaskRequest>,
    ) -> Result<Response<v1::RetryDevelopmentTaskResponse>, Status> {
        let context = self.context(&request, "RetryDevelopmentTask")?;
        let request = request.into_inner();
        let task = self
            .application
            .retry_development_task(
                context.idempotency_key(request.idempotency_key)?,
                &request.task_id,
            )
            .map_err(|error| context.error(error))?;
        context.ok(v1::RetryDevelopmentTaskResponse {
            task: Some(development_task(task)),
        })
    }

    async fn provide_development_task_input(
        &self,
        request: Request<v1::ProvideDevelopmentTaskInputRequest>,
    ) -> Result<Response<v1::ProvideDevelopmentTaskInputResponse>, Status> {
        let context = self.context(&request, "ProvideDevelopmentTaskInput")?;
        let request = request.into_inner();
        let task = self
            .application
            .provide_development_task_input(
                context.idempotency_key(request.idempotency_key)?,
                &request.task_id,
                &request.input,
            )
            .map_err(|error| context.error(error))?;
        context.ok(v1::ProvideDevelopmentTaskInputResponse {
            task: Some(development_task(task)),
        })
    }

    async fn get_development_task_evidence(
        &self,
        request: Request<v1::GetDevelopmentTaskEvidenceRequest>,
    ) -> Result<Response<v1::GetDevelopmentTaskEvidenceResponse>, Status> {
        let context = self.context(&request, "GetDevelopmentTaskEvidence")?;
        let request = request.into_inner();
        let evidence = self
            .application
            .get_development_task_evidence(&request.task_id)
            .map_err(|error| context.error(error))?;
        context.ok(v1::GetDevelopmentTaskEvidenceResponse {
            evidence: Some(development_evidence(evidence)),
        })
    }

    type WatchDevelopmentTaskStream = BoxGrpcStream<v1::WatchDevelopmentTaskResponse>;

    async fn watch_development_task(
        &self,
        request: Request<v1::WatchDevelopmentTaskRequest>,
    ) -> Result<Response<Self::WatchDevelopmentTaskStream>, Status> {
        let context = self.context(&request, "WatchDevelopmentTask")?;
        let deadline = request_deadline(&request);
        let request = request.into_inner();
        let initial = self
            .application
            .get_development_task(&request.task_id)
            .map_err(|error| context.error(error))?;
        let mut last_seen = request.after_resource_version.unwrap_or_default();
        let send_initial =
            request.after_resource_version.is_none() || initial.resource_version > last_seen;
        let initial_terminal = terminal_task(&initial);
        let application = self.application.clone();
        let task_id = request.task_id;
        let request_id = context.request_id.clone();
        let mut shutdown = self.shutdown.clone();
        let stream = async_stream::stream! {
            if deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
                yield Err(watch_deadline_status(&task_id, &request_id));
                return;
            }
            if send_initial {
                last_seen = initial.resource_version;
                yield Ok(watch_response(initial));
                if initial_terminal {
                    return;
                }
            } else if initial_terminal {
                return;
            }

            let mut interval = tokio::time::interval(WATCH_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let far_future =
                tokio::time::Instant::now() + std::time::Duration::from_secs(3_153_600_000);
            let deadline_sleep = tokio::time::sleep_until(deadline.unwrap_or(far_future));
            tokio::pin!(deadline_sleep);
            loop {
                tokio::select! {
                    biased;
                    () = &mut deadline_sleep => {
                        yield Err(watch_deadline_status(&task_id, &request_id));
                        return;
                    }
                    _ = interval.tick() => {
                        let task = match application.get_development_task(&task_id) {
                            Ok(task) => task,
                            Err(error) => {
                                yield Err(application_status(error, Some(&request_id)));
                                return;
                            }
                        };
                        if task.resource_version <= last_seen {
                            continue;
                        }
                        last_seen = task.resource_version;
                        let terminal = terminal_task(&task);
                        yield Ok(watch_response(task));
                        if terminal {
                            return;
                        }
                    }
                    result = shutdown.changed() => {
                        if result.is_err() || *shutdown.borrow() {
                            return;
                        }
                    }
                }
            }
        };
        Ok(context.stream(stream))
    }
}

include!("grpc_conversion.rs");

#[cfg(test)]
#[path = "grpc_tests.rs"]
mod tests;
