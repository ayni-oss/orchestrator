//! Authenticated observation ingress and debug-only loopback reads.
use ayni_orchestrator_api::v1::{
    self,
    debug_logs_service_server::DebugLogsService,
    execution_telemetry_service_server::{
        ExecutionTelemetryService, ExecutionTelemetryServiceServer,
    },
};
use ayni_orchestrator_events::runtime_preparation::{Binding, worker_token};
use ayni_orchestrator_workflow_postgres::WorkflowStore;
use std::{io, pin::Pin, sync::Arc, time::Duration};
use subtle::ConstantTimeEq as _;
use tokio::sync::{Semaphore, watch};
use tokio_stream::{Stream, StreamExt as _};
use tonic::{Request, Response, Status, transport::Server};

type Responses<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;
static STREAMS: Semaphore = Semaphore::const_new(64);
static WATCHES: Semaphore = Semaphore::const_new(8);

#[derive(Clone)]
pub struct Telemetry {
    store: WorkflowStore,
    debug: bool,
    collector_token: Option<Arc<str>>,
    runtime_token: Option<Arc<str>>,
    task_token: Option<Arc<str>>,
    publication_token: Option<Arc<str>>,
    shutdown: Option<watch::Receiver<bool>>,
}
impl Telemetry {
    #[must_use]
    pub fn new(store: WorkflowStore) -> Self {
        Self {
            store,
            shutdown: None,
            debug: debug_enabled(),
            collector_token: read_token("ORCHESTRATOR_TELEMETRY_TOKEN_PATH"),
            runtime_token: read_token("ORCHESTRATOR_RUNTIME_AUTHORITY_TOKEN_PATH"),
            task_token: read_token("ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH"),
            publication_token: read_token("ORCHESTRATOR_PUBLICATION_AUTHORITY_TOKEN_PATH"),
        }
    }
    #[must_use]
    pub fn with_shutdown(mut self, signal: watch::Receiver<bool>) -> Self {
        self.shutdown = Some(signal);
        self
    }
    fn require_debug(&self) -> Result<(), Status> {
        if self.debug && !self.shutdown.as_ref().is_some_and(|s| *s.borrow()) {
            Ok(())
        } else {
            Err(Status::permission_denied("debug logs disabled"))
        }
    }
}
fn read_token(name: &str) -> Option<Arc<str>> {
    std::env::var(name)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|token| Arc::from(token.trim()))
}
#[must_use]
pub fn debug_enabled() -> bool {
    std::env::var("ORCHESTRATOR_DEBUG").is_ok_and(|v| v == "true")
}
#[must_use]
pub fn retention_seconds() -> i64 {
    configured("ORCHESTRATOR_DEBUG_RETENTION_SECONDS", 86400)
}
#[must_use]
pub fn max_bytes() -> i64 {
    configured("ORCHESTRATOR_DEBUG_MAX_BYTES", 1024 * 1024 * 1024)
}
fn configured(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// Separate from the loopback-only workflow authority API.
/// # Errors
/// Return listener failures; no configured ingress means no external listener.
pub async fn serve(store: WorkflowStore, mut shutdown: watch::Receiver<bool>) -> io::Result<()> {
    let Ok(address) = std::env::var("ORCHESTRATOR_TELEMETRY_BIND_ADDRESS") else {
        let _ = shutdown.changed().await;
        return Ok(());
    };
    let api = Telemetry::new(store).with_shutdown(shutdown.clone());
    if api.collector_token.is_none()
        && api.runtime_token.is_none()
        && api.task_token.is_none()
        && api.publication_token.is_none()
    {
        return Err(io::Error::other("telemetry authority unavailable"));
    }
    Server::builder()
        .add_service(ExecutionTelemetryServiceServer::new(api).max_decoding_message_size(65536))
        .serve_with_shutdown(address.parse().map_err(io::Error::other)?, async move {
            let _ = shutdown.changed().await;
        })
        .await
        .map_err(io::Error::other)
}

#[tonic::async_trait]
impl ExecutionTelemetryService for Telemetry {
    type ExchangeTelemetryStream = Responses<v1::ExchangeTelemetryResponse>;
    async fn exchange_telemetry(
        &self,
        request: Request<tonic::Streaming<v1::ExchangeTelemetryRequest>>,
    ) -> Result<Response<Self::ExchangeTelemetryStream>, Status> {
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|v| v.len() <= 256)
            .ok_or_else(|| Status::unauthenticated("telemetry identity required"))?
            .to_owned();
        let collector = self
            .collector_token
            .as_ref()
            .is_some_and(|expected| bool::from(expected.as_bytes().ct_eq(token.as_bytes())));
        if !collector
            && self.runtime_token.is_none()
            && self.task_token.is_none()
            && self.publication_token.is_none()
        {
            return Err(Status::unauthenticated("telemetry identity invalid"));
        }
        let permit = STREAMS
            .try_acquire()
            .map_err(|_| Status::resource_exhausted("telemetry streams"))?;
        ayni_orchestrator_workflow_postgres::metrics::increment(
            "workflow_telemetry_connections_total",
        );
        let api = self.clone();
        let mut incoming = request.into_inner();
        let output = async_stream::try_stream! {
            let _permit=permit;
            while let Some(record)=incoming.next().await {
                let record=record?.record.ok_or_else(||Status::invalid_argument("record required"))?;
                let source=record.source.as_ref().ok_or_else(||Status::invalid_argument("source required"))?;
                if !record.data.is_empty()&&!api.debug {Err(Status::permission_denied("raw logs disabled"))?;}
                if collector {
                    api.require_debug()?;
                } else if source.component=="task-worker" {
                    let authority=api.task_token.as_ref().ok_or_else(||Status::unauthenticated("task authority missing"))?;
                    let expected=ayni_orchestrator_events::task_execution::worker_token(authority.as_bytes(),&source.execution_id,source.generation);
                    if !bool::from(expected.as_bytes().ct_eq(token.as_bytes())) {Err(Status::unauthenticated("task identity mismatch"))?;}
                    api.store.validate_log_task_binding(source).await.map_err(|_|Status::permission_denied("unknown task binding"))?;
                } else if source.component=="task-publisher" {
                    let valid=api.publication_token.as_ref().is_some_and(|expected|bool::from(expected.as_bytes().ct_eq(token.as_bytes())));
                    if !valid {Err(Status::unauthenticated("publisher identity mismatch"))?;}
                    if !source.execution_id.is_empty() {api.store.validate_log_publication_binding(source).await.map_err(|_|Status::permission_denied("unknown publication binding"))?;}
                } else {
                    let authority=api.runtime_token.as_ref().ok_or_else(||Status::unauthenticated("runtime authority missing"))?;
                    let expected=worker_token(authority.as_bytes(),&source.execution_id,&source.stage,source.generation,&record.command_digest);
                    if !bool::from(expected.as_bytes().ct_eq(token.as_bytes())) {Err(Status::unauthenticated("execution identity mismatch"))?;}
                    let binding=Binding{operation_id:source.execution_id.clone(),stage:source.stage.clone(),generation:source.generation,pod_uid:source.pod_uid.clone(),command_digest:record.command_digest.clone()};
                    // Exact retained binding permits final spool replay; observations cannot advance state.
                    api.store.validate_log_runtime_binding(&binding).await.map_err(|_|Status::permission_denied("unknown execution binding"))?;
                }
                let began=std::time::Instant::now();
                api.store.append_log_bounded(&record,max_bytes()).await.map_err(|error| match error {
                    ayni_orchestrator_workflow_postgres::StoreError::IdempotencyConflict => Status::already_exists("conflicting telemetry sequence"),
                    ayni_orchestrator_workflow_postgres::StoreError::InvalidInput(_) => Status::invalid_argument("invalid telemetry sequence"),
                    _ => { ayni_orchestrator_workflow_postgres::metrics::increment("workflow_telemetry_persistence_errors_total"); Status::unavailable("log persistence unavailable") }
                })?;
                ayni_orchestrator_workflow_postgres::metrics::observe("workflow_telemetry_commit_seconds",began.elapsed());
                let cancel_requested = !collector && api.store.log_cancellation(source).await.unwrap_or(false);
                yield v1::ExchangeTelemetryResponse{source_id:source.id.clone(),sequence:record.sequence,cancel_requested};
            }
        };
        Ok(Response::new(Box::pin(output)))
    }
}

#[tonic::async_trait]
impl DebugLogsService for Telemetry {
    async fn get_debug_capabilities(
        &self,
        _: Request<v1::GetDebugCapabilitiesRequest>,
    ) -> Result<Response<v1::GetDebugCapabilitiesResponse>, Status> {
        Ok(Response::new(v1::GetDebugCapabilitiesResponse {
            enabled: self.debug,
            retention_seconds: retention_seconds().unsigned_abs(),
            max_retained_bytes: max_bytes().unsigned_abs(),
            collection: "kubernetes_api; completeness unknown across rotation and abrupt deletion"
                .into(),
        }))
    }
    async fn list_log_sources(
        &self,
        request: Request<v1::ListLogSourcesRequest>,
    ) -> Result<Response<v1::ListLogSourcesResponse>, Status> {
        self.require_debug()?;
        let q = request.into_inner();
        let sources = self
            .store
            .log_sources(&q.execution_id, &q.after_id)
            .await
            .map_err(|_| Status::unavailable("source inventory unavailable"))?;
        let next_after_id = if sources.len() == 100 {
            sources.last().map(|s| s.id.clone()).unwrap_or_default()
        } else {
            String::new()
        };
        Ok(Response::new(v1::ListLogSourcesResponse {
            sources,
            next_after_id,
        }))
    }
    async fn read_logs(
        &self,
        request: Request<v1::ReadLogsRequest>,
    ) -> Result<Response<v1::ReadLogsResponse>, Status> {
        self.require_debug()?;
        Ok(Response::new(
            self.store
                .read_logs(&request.into_inner())
                .await
                .map_err(|_| Status::unavailable("log replay unavailable"))?,
        ))
    }
    type WatchLogsStream = Responses<v1::WatchLogsResponse>;
    async fn watch_logs(
        &self,
        request: Request<v1::WatchLogsRequest>,
    ) -> Result<Response<Self::WatchLogsStream>, Status> {
        self.require_debug()?;
        let permit = WATCHES
            .try_acquire()
            .map_err(|_| Status::resource_exhausted("log watches"))?;
        let api = self.clone();
        let store = self.store.clone();
        let mut q = request.into_inner().query.unwrap_or_default();
        let stream = async_stream::try_stream! {
            let _permit=permit;
            let deadline=tokio::time::Instant::now()+Duration::from_secs(60);
            while tokio::time::Instant::now()<deadline {
                api.require_debug()?;
                let page=store.read_logs(&q).await.map_err(|_|Status::unavailable("log replay unavailable"))?;
                let progressed=page.next_cursor>q.after;
                q.after=page.next_cursor;yield v1::WatchLogsResponse { page: Some(page) };
                if !progressed {tokio::time::sleep(Duration::from_millis(500)).await;}
            }
        };
        Ok(Response::new(Box::pin(stream)))
    }
}

/// Independently bounded raw retention and retained-evidence projection.
pub async fn maintain(store: WorkflowStore, mut shutdown: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=shutdown.changed()=>return,
            _=tick.tick()=>{
                if debug_enabled() { let _=store.mirror_execution_diagnostics(&std::env::var("ORCHESTRATOR_DEPLOYMENT").unwrap_or_default(),max_bytes()).await; }
                if store.cleanup_logs(retention_seconds(),max_bytes()).await.is_err() { ayni_orchestrator_workflow_postgres::metrics::increment("workflow_capture_cleanup_errors_total"); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn debug_off_rejects_reads_sources_and_watches_before_database_access() {
        let mut api = Telemetry::new(WorkflowStore::new("postgresql://invalid"));
        api.debug = false;
        assert_eq!(
            api.read_logs(Request::new(v1::ReadLogsRequest::default()))
                .await
                .err()
                .map(|e| e.code()),
            Some(tonic::Code::PermissionDenied)
        );
        assert_eq!(
            api.list_log_sources(Request::new(v1::ListLogSourcesRequest::default()))
                .await
                .err()
                .map(|e| e.code()),
            Some(tonic::Code::PermissionDenied)
        );
        assert_eq!(
            api.watch_logs(Request::new(v1::WatchLogsRequest::default()))
                .await
                .err()
                .map(|e| e.code()),
            Some(tonic::Code::PermissionDenied)
        );
        let (stop, receiver) = watch::channel(false);
        api.debug = true;
        let api = api.with_shutdown(receiver);
        stop.send_replace(true);
        assert!(api.require_debug().is_err());
    }
}
