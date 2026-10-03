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

const reasonNames: Partial<Record<ErrorReason, string>> = {
  [ErrorReason.ERROR_REASON_INVALID_IDENTIFIER]: "invalid_identifier",
  [ErrorReason.ERROR_REASON_INVALID_INPUT]: "invalid_input",
  [ErrorReason.ERROR_REASON_IDEMPOTENCY_KEY_REUSED]: "idempotency_key_reused",
  [ErrorReason.ERROR_REASON_RESOURCE_NOT_FOUND]: "resource_not_found",
  [ErrorReason.ERROR_REASON_CONFLICT]: "conflict",
  [ErrorReason.ERROR_REASON_FAILED_PRECONDITION]: "failed_precondition",
  [ErrorReason.ERROR_REASON_TASK_NOT_ACTIONABLE]: "task_not_actionable",
  [ErrorReason.ERROR_REASON_PROVIDER_UNAVAILABLE]: "provider_unavailable",
  [ErrorReason.ERROR_REASON_MODEL_UNAVAILABLE]: "model_unavailable",
  [ErrorReason.ERROR_REASON_REPOSITORY_ACCESS]: "repository_access",
  [ErrorReason.ERROR_REASON_EVIDENCE_UNAVAILABLE]: "evidence_unavailable",
  [ErrorReason.ERROR_REASON_CANCELLED]: "cancelled",
  [ErrorReason.ERROR_REASON_DEADLINE_EXCEEDED]: "deadline_exceeded",
  [ErrorReason.ERROR_REASON_INTERNAL]: "internal",
  [ErrorReason.ERROR_REASON_INVALID_REPOSITORY_REVISION]: "invalid_repository_revision",
  [ErrorReason.ERROR_REASON_INVALID_PROFILE_REVISION]: "invalid_profile_revision",
  [ErrorReason.ERROR_REASON_INVALID_LOCK_FINGERPRINT]: "invalid_lock_fingerprint",
  [ErrorReason.ERROR_REASON_INVALID_OCI_REFERENCE]: "invalid_oci_reference",
  [ErrorReason.ERROR_REASON_INVALID_OCI_DIGEST]: "invalid_oci_digest",
  [ErrorReason.ERROR_REASON_INVALID_PLATFORM]: "invalid_platform",
  [ErrorReason.ERROR_REASON_PROFILE_UNSUPPORTED]: "profile_unsupported",
  [ErrorReason.ERROR_REASON_PROFILE_NOT_READY]: "profile_not_ready",
  [ErrorReason.ERROR_REASON_RUNTIME_ARTIFACT_NOT_VERIFIED]: "runtime_artifact_not_verified",
  [ErrorReason.ERROR_REASON_AUTOMATION_NOT_READY]: "automation_not_ready",
  [ErrorReason.ERROR_REASON_TASK_DEADLINE_EXPIRED]: "task_deadline_expired",
  [ErrorReason.ERROR_REASON_CHANGE_REQUEST_HEAD_STALE]: "change_request_head_stale",
  [ErrorReason.ERROR_REASON_ASSESSMENT_REJECTED]: "assessment_rejected",
  [ErrorReason.ERROR_REASON_EVIDENCE_INVALID]: "evidence_invalid",
  [ErrorReason.ERROR_REASON_BOUND_EXCEEDED]: "bound_exceeded",
  [ErrorReason.ERROR_REASON_WATCH_CURSOR_EXPIRED]: "watch_cursor_expired",
  [ErrorReason.ERROR_REASON_UNKNOWN_ENUM_VALUE]: "unknown_enum_value",
  [ErrorReason.ERROR_REASON_NOT_IMPLEMENTED]: "not_implemented",
};

function reason(value: ErrorReason): string {
  return reasonNames[value] ?? "unspecified";
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
    case grpcStatus.UNIMPLEMENTED:
      return 501;
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
    case grpcStatus.UNIMPLEMENTED:
      return "not_implemented";
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
    case grpcStatus.UNIMPLEMENTED:
      return "the operation is not implemented by this control plane";
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
  const onboardingErrors: Record<string,string> = {
    github_app_unconfigured:"Your operator must configure the existing GitHub App in this deployment.",
    github_not_found:"The App cannot see this repository. Check its name and installation access.",
    github_authentication_failed:"GitHub App authentication failed. Ask your operator to check the deployment configuration.",
    github_access_denied:"The App lacks access. Review its repository installation and permissions.",
    repository_connection_conflict:"This repository is already connected or the request conflicts with retained state. Open its existing connection.",
  };
  const onboardingMessage=onboardingErrors[error.details];
  if(onboardingMessage) return new BffError({status:statusFor(error.code),reason:error.details,message:onboardingMessage,retryable:false,...(safeFallbackRequestId ? {requestId:safeFallbackRequestId} : {})});
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
