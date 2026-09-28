import { BinaryReader } from "@bufbuild/protobuf/wire";
import { status as grpcStatus, type Metadata, type ServiceError } from "@grpc/grpc-js";
import { ErrorDetail, ErrorReason } from "@ayni-orchestrator/grpc-bindings/server";

const ERROR_DETAIL_TYPE = "type.googleapis.com/ayni.orchestrator.v1.ErrorDetail";

export interface BrowserErrorDto {
  status: number;
  reason: string;
  message: string;
  requestId?: string;
  resourceType?: string;
  resourceId?: string;
  retryable: boolean;
}

export class BffError extends Error {
  readonly status: number;
  readonly reason: string;
  readonly requestId?: string;
  readonly resourceType?: string;
  readonly resourceId?: string;
  readonly retryable: boolean;

  constructor(value: BrowserErrorDto) {
    super(value.message);
    this.name = "BffError";
    this.status = value.status;
    this.reason = value.reason;
    this.retryable = value.retryable;
    if (value.requestId !== undefined) this.requestId = value.requestId;
    if (value.resourceType !== undefined) this.resourceType = value.resourceType;
    if (value.resourceId !== undefined) this.resourceId = value.resourceId;
  }

  toDto(): BrowserErrorDto {
    return {
      status: this.status,
      reason: this.reason,
      message: this.message,
      retryable: this.retryable,
      ...(this.requestId === undefined ? {} : { requestId: this.requestId }),
      ...(this.resourceType === undefined ? {} : { resourceType: this.resourceType }),
      ...(this.resourceId === undefined ? {} : { resourceId: this.resourceId }),
    };
  }
}

function bounded(value: string, maximum = 512): string {
  let candidate = value.replace(/[\r\n\0]/gu, " ");
  while (Buffer.byteLength(candidate, "utf8") > maximum) {
    candidate = candidate.slice(0, -1);
  }
  return candidate;
}

function safeRequestId(value: unknown): string | undefined {
  return typeof value === "string" && /^[A-Za-z0-9._-]{1,64}$/u.test(value) ? value : undefined;
}

function metadataRequestId(metadata: Metadata | undefined): string | undefined {
  const value = metadata?.get("x-request-id")[0];
  return safeRequestId(typeof value === "string" ? value : undefined);
}

function reason(value: ErrorReason): string {
  switch (value) {
    case ErrorReason.ERROR_REASON_INVALID_IDENTIFIER:
      return "invalid_identifier";
    case ErrorReason.ERROR_REASON_INVALID_INPUT:
      return "invalid_input";
    case ErrorReason.ERROR_REASON_IDEMPOTENCY_KEY_REUSED:
      return "idempotency_key_reused";
    case ErrorReason.ERROR_REASON_RESOURCE_NOT_FOUND:
      return "resource_not_found";
    case ErrorReason.ERROR_REASON_CONFLICT:
      return "conflict";
    case ErrorReason.ERROR_REASON_FAILED_PRECONDITION:
      return "failed_precondition";
    case ErrorReason.ERROR_REASON_TASK_NOT_ACTIONABLE:
      return "task_not_actionable";
    case ErrorReason.ERROR_REASON_PROVIDER_UNAVAILABLE:
      return "provider_unavailable";
    case ErrorReason.ERROR_REASON_MODEL_UNAVAILABLE:
      return "model_unavailable";
    case ErrorReason.ERROR_REASON_REPOSITORY_ACCESS:
      return "repository_access";
    case ErrorReason.ERROR_REASON_EVIDENCE_UNAVAILABLE:
      return "evidence_unavailable";
    case ErrorReason.ERROR_REASON_CANCELLED:
      return "cancelled";
    case ErrorReason.ERROR_REASON_DEADLINE_EXCEEDED:
      return "deadline_exceeded";
    case ErrorReason.ERROR_REASON_INTERNAL:
      return "internal";
    case ErrorReason.ERROR_REASON_INVALID_SOURCE_REVISION:
      return "invalid_source_revision";
    case ErrorReason.ERROR_REASON_INVALID_LOCK_DIGEST:
      return "invalid_lock_digest";
    case ErrorReason.ERROR_REASON_REPOSITORY_NOT_READY:
      return "repository_not_ready";
    case ErrorReason.ERROR_REASON_TASK_PHASE_NOT_ELIGIBLE:
      return "task_phase_not_eligible";
    case ErrorReason.ERROR_REASON_TASK_DEADLINE_EXPIRED:
      return "task_deadline_expired";
    default:
      return "unspecified";
  }
}

function parseAny(reader: BinaryReader): { typeUrl: string; value: Uint8Array } {
  const end = reader.uint32() + reader.pos;
  let typeUrl = "";
  let value = new Uint8Array();
  while (reader.pos < end) {
    const tag = reader.uint32();
    switch (tag >>> 3) {
      case 1:
        typeUrl = reader.string();
        break;
      case 2:
        value = new Uint8Array(reader.bytes());
        break;
      default:
        reader.skip(tag & 7);
    }
  }
  return { typeUrl, value };
}

export function decodeErrorDetail(bytes: Uint8Array): ErrorDetail | undefined {
  try {
    const reader = new BinaryReader(bytes);
    while (reader.pos < reader.len) {
      const tag = reader.uint32();
      if ((tag >>> 3) === 3) {
        const any = parseAny(reader);
        if (any.typeUrl === ERROR_DETAIL_TYPE) return ErrorDetail.decode(any.value);
      } else {
        reader.skip(tag & 7);
      }
    }
  } catch {
    return undefined;
  }
  return undefined;
}

function richDetail(error: ServiceError): ErrorDetail | undefined {
  const value = error.metadata?.get("grpc-status-details-bin")[0];
  if (Buffer.isBuffer(value)) return decodeErrorDetail(value);
  return undefined;
}

function statusFor(code: number): number {
  switch (code) {
    case grpcStatus.INVALID_ARGUMENT:
    case grpcStatus.OUT_OF_RANGE:
      return 400;
    case grpcStatus.UNAUTHENTICATED:
      return 401;
    case grpcStatus.PERMISSION_DENIED:
      return 403;
    case grpcStatus.NOT_FOUND:
      return 404;
    case grpcStatus.ALREADY_EXISTS:
    case grpcStatus.ABORTED:
      return 409;
    case grpcStatus.FAILED_PRECONDITION:
      return 412;
    case grpcStatus.RESOURCE_EXHAUSTED:
      return 429;
    case grpcStatus.CANCELLED:
      return 499;
    case grpcStatus.DEADLINE_EXCEEDED:
      return 504;
    case grpcStatus.UNAVAILABLE:
      return 503;
    default:
      return 500;
  }
}

function genericReason(code: number): string {
  switch (code) {
    case grpcStatus.INVALID_ARGUMENT:
    case grpcStatus.OUT_OF_RANGE:
      return "invalid_input";
    case grpcStatus.NOT_FOUND:
      return "resource_not_found";
    case grpcStatus.ALREADY_EXISTS:
      return "idempotency_key_reused";
    case grpcStatus.ABORTED:
      return "conflict";
    case grpcStatus.FAILED_PRECONDITION:
      return "failed_precondition";
    case grpcStatus.CANCELLED:
      return "cancelled";
    case grpcStatus.DEADLINE_EXCEEDED:
      return "deadline_exceeded";
    case grpcStatus.UNAVAILABLE:
      return "dependency_unavailable";
    case grpcStatus.UNAUTHENTICATED:
      return "unauthenticated";
    case grpcStatus.PERMISSION_DENIED:
      return "permission_denied";
    default:
      return "internal";
  }
}

function genericMessage(code: number): string {
  switch (code) {
    case grpcStatus.INVALID_ARGUMENT:
    case grpcStatus.OUT_OF_RANGE:
      return "the submitted request is invalid";
    case grpcStatus.NOT_FOUND:
      return "the requested resource was not found";
    case grpcStatus.ALREADY_EXISTS:
    case grpcStatus.ABORTED:
      return "the request conflicts with retained state";
    case grpcStatus.FAILED_PRECONDITION:
      return "the request cannot be applied in the current state";
    case grpcStatus.CANCELLED:
      return "the request was cancelled";
    case grpcStatus.DEADLINE_EXCEEDED:
      return "the control plane did not respond before the deadline";
    case grpcStatus.UNAVAILABLE:
      return "the control plane is temporarily unavailable";
    case grpcStatus.UNAUTHENTICATED:
      return "authentication is required";
    case grpcStatus.PERMISSION_DENIED:
      return "the operation is not permitted";
    default:
      return "the control plane request failed";
  }
}

function isServiceError(value: unknown): value is ServiceError {
  return value instanceof Error && typeof (value as Partial<ServiceError>).code === "number";
}

export function toBffError(error: unknown, fallbackRequestId?: string): BffError {
  if (error instanceof BffError) return error;
  const safeFallbackRequestId = safeRequestId(fallbackRequestId);
  if (!isServiceError(error)) {
    return new BffError({
      status: 500,
      reason: "internal",
      message: "the browser server could not complete the request",
      ...(safeFallbackRequestId === undefined ? {} : { requestId: safeFallbackRequestId }),
      retryable: false,
    });
  }
  const detail = richDetail(error);
  const requestId = safeRequestId(detail?.requestId)
    ?? metadataRequestId(error.metadata)
    ?? safeFallbackRequestId;
  const mappedReason = detail === undefined ? genericReason(error.code) : reason(detail.reason);
  const message = detail === undefined || detail.message.length === 0
    ? genericMessage(error.code)
    : bounded(detail.message);
  return new BffError({
    status: statusFor(error.code),
    reason: mappedReason,
    message,
    retryable: error.code === grpcStatus.UNAVAILABLE || error.code === grpcStatus.DEADLINE_EXCEEDED,
    ...(requestId === undefined ? {} : { requestId }),
    ...(detail?.resourceType === undefined ? {} : { resourceType: bounded(detail.resourceType, 64) }),
    ...(detail?.resourceId === undefined ? {} : { resourceId: bounded(detail.resourceId) }),
  });
}
