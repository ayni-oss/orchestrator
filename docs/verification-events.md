# Runtime verification event contract

The `ayni.orchestrator.events.v1` contract describes a supervisor observation for
one runtime verification execution. The shared Rust library in
`backend/libs/events` validates and encodes structured CloudEvents 1.0 JSON with
generated ProtoJSON payloads. TypeScript bindings come from the same schema.
Reporting authority is frozen per preparation generation. Legacy and shadow
generations retain their reader; new event-owned generations use authenticated
receipts for acceptance. Receipt alone does not activate a runtime. Both acceptance
paths share the guarded transaction for runtime projection, journal and outbox.

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
`ayni.build.orchestrator.runtime.verification.result.available.v1`, its dataschema is
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
validated on reopen and drained in batches of at most 128 on startup and every
five seconds. Receipt and shadow decisions produce no resource events or runtime
transitions. Unavailable evidence is deferred for thirty seconds, so it cannot
starve other receipts. The inbox retains at most 100000 observations and rejects
new receipts under pressure; it never evicts unprocessed evidence. Ingress limits
body size to 16384 bytes and concurrent authenticated requests to 32.

The supervisor fsyncs `verification-event.json` before sending and preserves its
identity and exact bytes on process replay, with the historical type-only
normalization described in [the shared contract](cloudevents-contract.md). It retries transport errors, 408, 429
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

Evidence is read outside the SQLite write transaction. The shadow consumer checks
its bounded retained record, exact size/hash, evidence contract, Pod/request binding
and outcome before applying current Ayni evidence policy. Inside the transaction
it rechecks the selected revision, frozen generation, request, source, lock,
policy, platform, digest, failure/cancellation authority and execution deadline.
A valid shadow result becomes `shadow_accepted`; invalid or stale results become
`rejected` with a bounded reason. For retained and shadow generations, both are durable internal decisions without
activation, failure accounting or outgoing accepted events. Event-owned receipts
remain pending for source-aware reconciliation; valid physical failures are
accepted with guarded failure accounting in the inbox transaction. Matching delivery
replays still receive the original stored acknowledgement after processing.

Schema 12 adds `verification_journal` and `verification_outbox`. The retained reader owns legacy/shadow activation; event-owned generations use
their authenticated receipt. Its admitted report commits
runtime/artifact projection, client resource events, immutable accepted decision,
stable CloudEvents outbox bytes and the retained cleanup intent together under
one immediate SQLite transaction. Resource mutations use nested savepoints inside
that transaction. The existing preparation cursor retains the report and pending
cleanup; there is no additional scheduler. Cleanup runs after commit, and its
completion compares the cursor again so it cannot overwrite a replacement
generation. A crash resumes the same cleanup intent and journal/outbox identity.
A failed quality check can still establish a valid signed runtime under the
existing policy; shadow processing never counts a physical failure.

The accepted event is
`ayni.build.orchestrator.runtime.verification.accepted.v1`, with generated
`VerificationDecisionRecorded` ProtoJSON, a controller-owned source, selected
revision, preparation/execution identity, aggregate revision and canonical report
SHA256. Observation causation links event-owned acceptance to the original stored
source/ID; it is absent for retained-reader admission.
Schema 14 persists retry scheduling, bounded attempts, acknowledgement timestamp
and broker sequence in the outbox. Acceptance rejects atomically at 100000 pending
publications; acknowledged history no longer consumes that admission budget.
Startup validates indexed identities, typed payloads, delivery bookkeeping and
the journal/outbox relationship. The opt-in local publisher sends at most eight
due immutable events per pass and waits for the expected stream's ACK before
marking delivery. Broker failures never reverse accepted decisions.

Acknowledged journal/outbox pairs expire after 30 days, in batches of 128, only
once their preparation and execution are absent from the active cursor. Pending
publications and reachable cleanup remain retained. Pruned verification replays
still fail stale cursor/generation guards. Inbox retention remains separately
bounded at 100000 observations. See [publisher operations](local-development.md#persistent-local-jetstream)
for retry timing, metrics and the finite broker deduplication window. Durable
the first durable broker consumer wakes committed verification cleanup through the existing reconciler.

Each execution has one terminal observation, so verification needs no streaming
sequence protocol or second scheduler. Bounded payloads and separate evidence
allow ingress and consumers to scale independently; partitioning can follow the
execution/resource key without inventing broker ordering authority. Retained and shadow generations keep their reader while they drain. Retention,
broker quarantine and removal of compatibility readers require separate gates.
Other runtime preparation flows use the [runtime operation contract](runtime-operation-events.md).

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
Set `runtime.verificationMode` to `events` to select event authority for newly
admitted preparation generations; it defaults to `shadow`. Event mode requires
reporting transport. Schema 13 freezes mode, endpoint and producer source in each
preparation identity before worker admission. Missing fields in older cursors
mean retained authority. Updating mode, source or worker defaults never changes
admitted work, including retries in the same generation. Authenticated ingress
uses the immutable admission manifest's source so old producer identities can drain.
Keep the original endpoint reachable and reporting RBAC enabled until its work
has drained. Disabling the transport is not a rollback procedure.

Event-owned verification looks for stored authenticated evidence before inspecting
Job completion. It reads and validates retained evidence outside the write
transaction and preserves existing fresh-source, certificate, lock, platform and
image checks. The final transaction rechecks the selected revision and generation,
accepts the inbox receipt, commits Ready and stores the causally linked journal and
outbox with cleanup intent. Missing receipts after process exit remain reporting
errors; this handler never falls back to the retained reader. The existing deadline
can still terminate an execution. Invalid requests and reporting-delivery failures
retain their operational retry classification without consuming the physical
execution budget. Physical failure events count once and record
their accepted receipt and cleanup intent atomically without creating a Ready fact.

Committed verification resumes cleanup without reinterpreting the accepted report
under new provider observations. Foreground deletion must finish before the intent
is cleared or another operation starts. Cleanup failure keeps the accepted Ready
state visible, and completion uses the cursor comparison guard. Late cancelled,
superseded or expired receipts are rejected and cannot reactivate work.

For rollback, restore `runtime.verificationMode: shadow` for subsequent generation
admission while event-owned generations finish under their frozen authority. Run
`python3 scripts/operations/verification-drain.py DATABASE` against the operator
store or a quiesced backup to enumerate reachable generation dependencies and
receipt counts. Zero active Pods is insufficient: resumable preparations without
a Pod still appear. Zero legacy dependencies does not authorize deletion of
historical evidence or readers; audit historical decoding before retirement.
Older binaries reject schema 16, so rollback requires a compatible binary.

`dev/tests/verification-reporting.sh` runs the local chart in a separate synthetic
namespace with Docker Desktop, tests real projected-token reporting, durable shadow
rejection for an unselected synthetic execution, and supervisor
replay across a control-plane restart against its retained SQLite store, using
the same physical Pod and forcing a duplicate HTTP delivery. Its optional `events`
argument seeds only a new disposable namespace with synthetic admitted state,
then verifies authoritative failure acceptance while the supervisor is still
running, durable once-only failure accounting and restart replay. It
stops the deployment and preserves its synthetic database/evidence for inspection.
The authentication design follows the Kubernetes
[bound service-account token and TokenReview contract](https://kubernetes.io/docs/reference/access-authn-authz/service-accounts-admin/).

The reporting fixture now removes its synthetic namespace and PVCs on success or failure.
