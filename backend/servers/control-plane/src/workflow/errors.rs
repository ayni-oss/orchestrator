//! Request correlation and safe typed gRPC error projection.
use super::{StoreError, v1, valid_request_id, validation};
use prost::Message as _;
use prost_types::Any;
use tonic::{Code, Request, Response, Status};
use uuid::Uuid;

const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail";

#[derive(Clone)]
pub(super) struct RequestContext {
    pub(super) request_id: String,
}

impl RequestContext {
    pub(super) fn new<T>(request: &Request<T>) -> Self {
        let request_id = request
            .metadata()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_request_id(value))
            .map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
        Self { request_id }
    }

    pub(super) fn response<T>(&self, value: T) -> Response<T> {
        let mut response = Response::new(value);
        if let Ok(value) = self.request_id.parse() {
            response.metadata_mut().insert("x-request-id", value);
        }
        response
    }

    pub(super) fn validation(&self, error: &validation::ValidationError) -> Status {
        self.status(
            Code::InvalidArgument,
            error.reason,
            "workflow request validation failed",
        )
    }

    pub(super) fn store(&self, error: &StoreError) -> Status {
        match error {
            StoreError::InvalidInput(_) => self.status(
                Code::InvalidArgument,
                v1::ErrorReason::InvalidInput,
                "workflow request validation failed",
            ),
            StoreError::NotFound => self.status(
                Code::NotFound,
                v1::ErrorReason::ResourceNotFound,
                "workflow resource was not found",
            ),
            StoreError::IdempotencyConflict => self.status(
                Code::AlreadyExists,
                v1::ErrorReason::IdempotencyConflict,
                "idempotency key conflicts with the retained request",
            ),
            StoreError::Capacity(_) => self.status(
                Code::ResourceExhausted,
                v1::ErrorReason::BoundExceeded,
                "workflow capacity is exhausted",
            ),
            StoreError::WatchCursorExpired => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::WatchCursorExpired,
                "watch cursor expired; reconnect without a cursor for a snapshot",
            ),
            StoreError::HistorySnapshotExpired => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::HistorySnapshotExpired,
                "history snapshot expired",
            ),
            StoreError::CorruptState => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::FailedPrecondition,
                "retained workflow state is invalid",
            ),
            StoreError::TaskAlreadyPrepared => self.status(
                Code::AlreadyExists,
                v1::ErrorReason::TaskAlreadyPrepared,
                "task already has a prepared attempt; read its execution preparation",
            ),
            StoreError::PlanAlreadyApproved => self.status(
                Code::AlreadyExists,
                v1::ErrorReason::PlanAlreadyApproved,
                "plan was already approved; read the retained approval",
            ),
            StoreError::PlanDigestMismatch => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::PlanDigestMismatch,
                "reviewed plan digest does not match the exact version",
            ),
            StoreError::PlanUnresolvedScope => self.status(
                Code::FailedPrecondition,
                v1::ErrorReason::PlanUnresolvedScope,
                "resolve scope questions and task acceptance criteria before approval",
            ),
            StoreError::PlanVersionConflict => self.status(
                Code::Aborted,
                v1::ErrorReason::PlanVersionConflict,
                "plan version changed; review the current version",
            ),
            StoreError::ConfigurationRevisionConflict => self.status(
                Code::Aborted,
                v1::ErrorReason::FailedPrecondition,
                "configuration revision changed",
            ),
            StoreError::Database(_) => self.status(
                Code::Unavailable,
                v1::ErrorReason::Internal,
                "authoritative workflow state is unavailable",
            ),
        }
    }

    pub(super) fn status(&self, code: Code, reason: v1::ErrorReason, message: &str) -> Status {
        let detail = v1::ErrorDetail {
            reason: reason as i32,
            message: message.to_owned(),
            resource_type: None,
            resource_id: None,
            metadata: Vec::new(),
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
        if let Ok(value) = self.request_id.parse() {
            status.metadata_mut().insert("x-request-id", value);
        }
        status
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct RichStatus {
    #[prost(int32, tag = "1")]
    pub(super) code: i32,
    #[prost(string, tag = "2")]
    pub(super) message: String,
    #[prost(message, repeated, tag = "3")]
    pub(super) details: Vec<Any>,
}
