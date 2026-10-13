# Runtime preparation reporting

New preparations admitted with `runtime.verificationMode: events` and configured
reporting freeze event authority for runtime image discovery, official builder
resolution, trust preparation, build and publication. Runtime verification uses
its existing [verification contract](verification-events.md). Earlier preparations
without `operation_reporting` retain their original readers, even when operator
defaults change. Connection onboarding and credential-probe migration remain
separate connection flows.

## Result and decision

The trusted supervisor retains `runtime-operation.json` in the execution's
subdirectory of the runtime-output claim. It binds the preparation, repository,
profile/revision, source, lock, policy, platform, physical Pod UID and request
identity. The request digest covers public operation inputs and the original Job
spec. Operation-specific frozen inputs include registry/image references, builder
identity, public certificate identity and publication candidate/overwrite intent.
The supervisor retains a typed discovery, trust, build, publication or safe failure
outcome. Trust policy and lock documents remain outside the event envelope.

The Protobuf `RuntimeOperationResultAvailable` payload references the exact retained
bytes by SHA-256 and size. `runtime-operation-v1` records are bounded to 2,200,000
bytes; the CloudEvents envelope remains bounded to 16,384 bytes. Evidence keys
are fixed opaque identifiers, never paths or fetch URLs. The controller resolves
only the execution-scoped retained file, rejects directory/file symlinks and
validates size, hash, physical binding, contract and outcome kind before reading
it as authority.

Supervisors deliver to the configured `/events/verification` internal endpoint,
which dispatches the typed verification and runtime-operation contracts. A projected
Pod-bound credential, TokenReview, controller Job UID and immutable admission
ConfigMap authenticate every binding and the operation kind. Repository children
have no reporting credential. Trust-policy tools use the existing mandatory
repository sandbox. Build/publication tools keep their existing admitted authority;
repository build execution remains inside the isolated engine.

SQLite schema 16 adds `runtime_operation_inbox`. Receipt commits before returning
202 `stored`; it establishes no preparation transition. Source/ID and execution
identity deduplicate decoded payloads. Changed contents, a replacement Pod or
another terminal identity for the same execution conflict. The inbox is bounded
to 100,000 records and fails closed under pressure.

The source-aware runtime reconciler reads evidence outside the write transaction.
Acceptance rechecks current generation, selected profile revision, frozen inputs,
producer source, execution ownership and failure/retry authority. Its immediate
transaction saves receipt acceptance, the preparation cursor, resource projection
and immutable `RuntimeOperationDecisionRecorded` outbox bytes together. The cursor
retains the dependent action and cleanup intent; there is no second scheduler.
Physical build/trust failures retain their accounting and profile failure in the
same transaction. Cancellation, replacement and exhausted generations cannot
advance through a late result.

## Progression and recovery

Accepted discovery distinguishes registry access failure from image absence.
Official builder resolution retains the selected manifest before cleanup. Accepted
trust output still undergoes policy/lock validation, reviewed provider merge and
current policy revalidation. A build result freezes its candidate while retaining
the engine through publication. Publication results, terminal exits without a
report and disappeared publisher Jobs all require independent registry readback.
Neither publisher success nor broker acknowledgement establishes runtime readiness.

Results can arrive before process exit. Missing terminal trust/build evidence is a
reporting error, rather than an inferred successful or failed execution. Verification
activation remains guarded by certificate, source, lock, platform and image bindings;
failed source quality remains a separate outcome. Committed verification cleanup
resumes before fresh provider lookup and cannot hide an accepted Ready result.
Foreground deletion and cursor comparison still gate replacement and credential
release.

The supervisor fsyncs the retained record and event before delivery and retries the
same bytes/identity until durable receipt, with historical type-only normalization
at the delivery boundary described in [the shared contract](cloudevents-contract.md). `report-operation` resumes delivery under
the original admission and Pod identity. Replaying a process with retained terminal
state resumes reporting rather than rerunning build, trust or publication. Reporting
reuses the bounded verification delivery implementation, including token rotation,
redirect rejection and acknowledgement validation.

The local publisher distributes accepted operation facts on
`orchestrator.runtime.operation.accepted.v1` in `ORCHESTRATOR_LIFECYCLE`. It shares
the eight-record publication budget with verification, persists retry scheduling
before network I/O and marks delivery only after the expected stream acknowledges
storage. Broker outage preserves accepted state and outgoing identities. Backlog
metrics include both flows. Acknowledged operation history expires after 30 days
only when its preparation is no longer reachable; pending publications remain.

For rollback, change subsequent admission defaults while retaining the original
endpoint/RBAC until frozen work drains. The read-only
`scripts/operations/verification-drain.py` reports both verification and runtime
operation reader dependencies for schemas 13–18, including trust review
continuations and repository-removal cleanup. It also inventories coding
admissions, pending receipts, conservative publication-recovery candidates and
retained history counts. See [drain inventory](../operations/operations.md#compatibility-drain-inventory)
for the report boundaries. Zero live Pods does not establish
zero resumable or historical dependencies. Compatibility-reader retirement remains
subject to the [migration gates](../operations/event-compatibility.md).

## Local validation

Run `dev/tests/verification-reporting.sh CONTROL_PLANE_IMAGE PINNED_WORKER_IMAGE
WEB_IMAGE events` with built images loaded into Docker Desktop. The disposable
Helm fixture exercises real discovery, trust, build and publication supervisors
with invalid public inputs, checks receipt before exit, restarts the controller
and replays the original bytes through that canonical delivery boundary. A seeded accepted runtime decision separately
exercises the production publisher and its real JetStream storage acknowledgement.
These checks cover transport and recovery; live GitHub trust proposals and GHCR
build/publication round trips still require an admitted provider fixture.

SQLite tests cover acceptance rollback, stale/cancelled/exhausted generations,
failure accounting, restart and publication replay. Worker tests cover large trust
documents above the termination-message limit and retained-result conflicts.
