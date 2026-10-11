# Runtime verification event contract

The `ayni.orchestrator.events.v1` contract describes a supervisor observation for
one runtime verification execution. The shared Rust library in
`backend/libs/events` validates and encodes structured CloudEvents 1.0 JSON with
generated ProtoJSON payloads. TypeScript bindings come from the same schema.
The current retained verification reader remains authoritative. No deployed
producer or reporting endpoint consumes this contract yet.

## Flow and authority

Admission owns the repository connection, runtime profile and profile revision,
preparation ID, frozen source revision, lock and policy digests, platform,
verification request digest and resolved image digest. A physical Kubernetes Job
and Pod UID bind the observation to one execution of that preparation. These
fields must come from trusted admission metadata supplied to the supervisor;
repository-controlled children cannot choose them or hold reporting credentials.
The existing request alone does not contain all these bindings. Reporting
integration must supply them explicitly rather than infer them from a report.

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
of reporting integration, not guarantees made by the codec. A newly admitted
retry has a new execution and event identity. Deduplication must reject conflicting
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

The next integration boundary is authenticated durable receipt: validate source
against supervisor credentials and admitted execution, store inbox content with
unique source/ID, then acknowledge and retry delivery independently of controller
availability. A receipt means stored, not accepted. Evidence reads stay outside
the SQLite acceptance transaction; current generation, selection, cancellation
and deadline must be rechecked inside it. Acceptance decision, state transition,
resource projection, dependent durable actions and outbox writes belong in one
transaction using the existing preparation cursor and action mechanisms.

Each execution has one terminal observation, so verification needs no streaming
sequence protocol or second scheduler. Bounded payloads and separate evidence
allow ingress and consumers to scale independently; partitioning can follow the
execution/resource key without inventing broker ordering authority. Retention,
retry budgets and quarantine are durable ingress concerns. A later rollout must
freeze the reporting contract at admission, keep old executions on the retained
reader while they drain, and ensure shadow observations never advance state.
NATS deployment and other flow migrations are outside this contract.

See [CloudEvents boundaries](cloudevents-contract.md),
[implementation proposal](event-platform-implementation.md) and
[migration requirements](event-platform-migration.md).
