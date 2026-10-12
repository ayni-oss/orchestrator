# Runtime verification event contract

The `ayni.orchestrator.events.v1` contract describes a supervisor observation for
one runtime verification execution. The shared Rust library in
`backend/libs/events` validates and encodes structured CloudEvents 1.0 JSON with
generated ProtoJSON payloads. TypeScript bindings come from the same schema.
The retained verification reader remains authoritative. Opt-in shadow reporting
adds authenticated durable receipt; receipt does not activate a runtime.

## Flow and authority

Admission owns the repository connection, runtime profile and profile revision,
preparation ID, frozen source revision, lock and policy digests, platform,
verification request digest and resolved image digest. A physical Kubernetes Job
and Pod UID bind the observation to one execution of that preparation. These
fields come from the immutable `reporting.json` ConfigMap manifest supplied to
the supervisor;
repository-controlled children cannot choose them or hold reporting credentials.
The coordinator supplies these bindings from frozen preparation and profile
state, alongside the existing verification request. The Kubernetes adapter binds
the request digest to the serialized request and creates the manifest before the
Job. The downward API supplies the physical Pod UID.

The supervisor writes the existing bounded `retained-v1` verification record
outside child-writable storage. It hashes the exact retained bytes and emits
`report_available`, or emits `execution_failed` with a recognized failure code
and an optional retained diagnostic reference. Report availability says nothing
about quality, certificate validity or runtime readiness. Controller acceptance
still requires reading and validating evidence against the frozen request and
current desired state, including effective certificate identity and selection.
Certificate identity remains in trusted admission state and is checked against
evidence; a producer assertion cannot replace the controller's certificate checks.

Evidence IDs are opaque local keys consisting of ASCII letters, digits, hyphens
or underscores, at most 128 bytes. They are not paths, URLs or fetch instructions.
A future evidence resolver must scope them to the authenticated execution,
resolve only supervisor-owned retained records, enforce the size bound before
reading and verify the declared SHA-256 before interpreting the record. The
reference contract is exactly `retained-v1`, with 1–131072 bytes. Lifecycle retention
must preserve records while their receipt or acceptance remains recoverable.

## Identity and wire behavior

The supervisor must persist a UUID event ID and its configured source
`ayni://orchestrator/supervisors/<producer-id>` before its first delivery. The
source and ID pair identifies a receipt; retries and restart replay reuse both,
including the observation content. The library accepts that identity from its
caller and never generates it. Durable storage and replay are responsibilities
of the supervisor delivery implementation, not guarantees made by the codec.
A newly admitted retry has a new execution and event identity. Deduplication must reject conflicting
content under the same source/ID; the pair alone grants no execution authority.

The event type is
`io.ayni.orchestrator.runtime.verification.result.available.v1`, its dataschema is
`ayni://orchestrator/schemas/runtime-verification-result/v1`, and its data content
type is `application/json`. The subject identifies the repository and runtime
profile; `operationid` and `executionid` extensions must match the body. The
codec restricts the entire envelope to 16384 bytes before parsing, rejects
non-JSON data and unsupported versions, and validates required identifiers,
digests and supported Linux platforms. It performs structural checks, not
comparison against trusted admission state or source authentication.

Protobuf is the payload source of truth, separate from resource API snapshots.
Generated ProtoJSON uses named enums and string `uint64` sizes; known numeric
enums are also accepted. Missing message fields retain absence, while unknown
or unspecified failure codes and missing or multiple outcomes fail validation.
Payload decoding rejects unknown fields; adding fields therefore requires a
coordinated reader upgrade before a producer sends them, or a new supported
schema version. Unknown bounded CloudEvents extensions are diagnostic metadata
and do not affect acceptance. Consumers must not use raw JSON or Protobuf
serialization as an authority token or ordering mechanism. Shared fixtures test
Rust/TypeScript round trips and presence; generated TypeScript conversion is
not an untrusted ingress validator.

## Recovery and scaling boundaries

Shadow reporting posts to `/events/verification` on the internal operational HTTP
listener with `application/cloudevents+json`. Kubernetes `TokenReview` validates
an audience of `ayni-orchestrator-verification`, the Pod-bound identity and its
service account. The ingress reads the Pod, checks its controller Job UID and
compares every binding against that Job's immutable admission manifest. Only the
supervisor mounts `/reporting/token`; checkout and repository children receive
no reporting credentials. Kubernetes lookup outages return a retryable 503.

SQLite schema 11 adds `verification_inbox`. Receipt uses a transaction with FULL
synchronous durability and unique source/ID and execution/Pod indexes. Duplicate
content is compared as decoded Protobuf values; JSON ordering and diagnostic
CloudEvents extensions do not create new observations. Conflicting content or a
second terminal identity for one physical execution returns 409. A matching
duplicate gets the same 202 `stored` receipt. The acknowledgement follows commit,
including when a disconnected caller causes delivery replay. Stored payloads are
validated on reopen and exposed through a bounded 128-row pending read; there is
no acceptance consumer yet. Inbox operations produce no resource events or
runtime transitions. The inbox retains at most 100000 observations and rejects
new receipts under pressure; it never evicts unprocessed evidence. Ingress limits
body size to 16384 bytes and concurrent authenticated requests to 32.

The supervisor fsyncs `verification-event.json` before sending and preserves its
identity and exact bytes on process replay. It retries transport errors, 408, 429
and 5xx responses with delays from one to thirty seconds. It rereads the projected
token on each attempt to support rotation and rejects redirects. A bounded,
matching 202 receipt is fsynced as `verification-receipt.json`. Permanent rejection
retains evidence and the event and reports a delivery error. Retry does not run
verification again. The Job's existing execution deadline bounds live delivery;
cancellation or deadline cleanup can stop delivery, but retained records survive.
`report-verification` resumes delivery from trusted mounted admission and retained
state, requiring the original Pod identity and credentials. It cannot move an old
observation to a new Pod. A durable receipt means stored, not accepted.

Opt-in Jobs wait for supervisor termination and its matching local receipt before
the retained reader observes their result. This prevents normal completion cleanup
from racing delivery. Old Jobs lack the shadow annotation and preserve their
reader behavior. Reporting failures are operational errors, separate from the
retained verification outcome. Evidence and inbox retention still require a
subsequent acceptance/retention policy; the receipt slice does not delete them.

Evidence reads stay outside the SQLite acceptance transaction; current generation, selection, cancellation
and deadline must be rechecked inside it. Acceptance decision, state transition,
resource projection, dependent durable actions and outbox writes belong in one
transaction using the existing preparation cursor and action mechanisms.

Each execution has one terminal observation, so verification needs no streaming
sequence protocol or second scheduler. Bounded payloads and separate evidence
allow ingress and consumers to scale independently; partitioning can follow the
execution/resource key without inventing broker ordering authority. Acceptance decisions, retention and quarantine remain subsequent controller
integration concerns. A later rollout must
freeze the reporting contract at admission, keep old executions on the retained
reader while they drain, and ensure shadow observations never advance state.
NATS deployment and other flow migrations are outside this contract.

See [CloudEvents boundaries](cloudevents-contract.md),
[implementation proposal](event-platform-implementation.md) and
[migration requirements](event-platform-migration.md).

## Operator configuration

Reporting defaults to `null`. To enable controlled shadow executions, set
`runtime.reporting.endpoint` to the trusted internal receipt URL and
`runtime.reporting.source` to a stable deployment producer source. The local
example uses `http://control-plane:8080/events/verification`; production transport
must protect the projected bearer credential, using HTTPS or a trusted encrypted
cluster network. Endpoint and source are operator-owned, never repository input.
The Helm chart adds only `create tokenreviews` cluster authority when enabled.
Keep the producer source stable while opted-in executions are pending. The
manifest freezes reporting at Job creation; admission-time generation selection,
controlled cutover, late cancelled receipts and shadow acceptance auditing remain
later integration work.

`dev/tests/verification-reporting.sh` runs the local chart in a separate synthetic
namespace with Docker Desktop, tests real projected-token reporting and supervisor
replay, then restarts the control plane against its retained SQLite store. It
stops the deployment and preserves its synthetic database/evidence for inspection.
The authentication design follows the Kubernetes
[bound service-account token and TokenReview contract](https://kubernetes.io/docs/reference/access-authn-authz/service-accounts-admin/).
