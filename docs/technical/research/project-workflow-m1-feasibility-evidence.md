# Project workflow M1 feasibility evidence

Date: 2026-10-20. Status: partial evidence bundle; maintainer review and the
remaining gates are required before runtime authority implementation.

This document retains reproducible evidence for the gates in
[the M1 durable-foundation contract](project-workflow-m1-durable-foundation.md).
It does not claim that M1, or all pre-implementation gates, passed.

## PostgreSQL client and migration stack

### Source and environment

- Repository: `ayni-oss/orchestrator`.
- Retained implementation revision:
  `a0b2633bb113adc371143929ff308e195ef64f94`. The later evidence-only commit
  changes no executable proof, protocol contract, fixture, dependency, or generated
  binding.
- PostgreSQL image: `postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3`.
- Image-reported version: PostgreSQL 17.6, Debian package `17.6-2.pgdg12+1`.
- Exercised platform: `linux/arm64` under Docker Desktop client/server 29.8.2.
- Rust: `rustc 1.94.0`; Cargo 1.94.0.
- Candidate client: `tokio-postgres 0.7.18`, plaintext `NoTls` against a
  loopback-only disposable database.
- Candidate migration mechanism: repository-owned forward SQL inside a
  transaction guarded by `pg_advisory_xact_lock`.

The test creates an ephemeral container with a random loopback port, uses only a
fixture password, and removes the container on every exit path. It does not read
`dev/.env`, Kubernetes Secrets, or external-service credentials.

### Reproduction

From the repository root with a healthy Docker daemon:

```sh
scripts/ci/check-workflow-postgres-feasibility.sh
```

The command pulls the image by digest and runs the ignored integration test in
`backend/servers/control-plane/crates/workflow-postgres/tests/feasibility.rs`.
The migration portion of the retained successful result was:

```text
running 1 test
postgres-feasibility: advisory-lock=pass rollback=pass concurrent-migration=pass notification-replay=pass fifo-row-lock=pass exact-outbox=pass
test postgres_feasibility_gate ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
postgres-image=postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3
postgres-platform=linux/arm64
```

### Observations

1. A separately held advisory lock prevented the migration transaction from
   reaching DDL until the lock was released. The test first observed the named
   migration backend waiting on the exact ungranted advisory-lock key in
   `pg_stat_activity`/`pg_locks`, so scheduler or connection-startup delay cannot
   satisfy the assertion.
2. Eight simultaneous migration callers completed, while the migration ledger
   retained exactly one version row.
3. An explicitly rolled-back authority transaction left no partial conversation
   row.
4. Sixteen competing admissions used `SELECT ... FOR UPDATE` on one conversation
   row and allocated exactly the contiguous FIFO range 1 through 16.
5. Event bytes containing NUL and non-UTF-8 octets survived insertion and replay
   through a fresh connection without transformation.
6. A notification emitted while the listener was disconnected was recovered by
   replaying the durable event table after the prior cursor. A subsequent
   `LISTEN` notification woke the live connection and matched its committed row.

### Limitations

- This is a feasibility schema, not the runtime authority schema or a migration
  from deployed data. Its single version marker does not yet checksum SQL or reject
  edited/applied migration drift.
- The run used one local PostgreSQL node, one architecture, plaintext loopback
  transport, and no pooler, failover, disk-full, or network-partition injection.
- The test proves transaction rollback, not recovery from a process kill at every
  statement boundary.
- The base migration test uses an unbounded fixture table. Synthetic retention,
  cursor expiry, slow readers, and Tonic behavior are exercised separately in the
  snapshot/watch prototype below; calendar-time seven-day pruning remains runtime work.
- FIFO allocation proves row-lock serialization but not the full production
  eligibility state machine.
- The image index digest and crate remain candidates until this bundle is
  reviewed; this evidence does not authorize runtime implementation by itself.

### Decision effect

This result supports `tokio-postgres 0.7.18`, transaction-scoped advisory-lock
migrations, durable table replay after `LISTEN` loss, row-lock FIFO admission, and
bytea storage of exact outbox events as viable candidates. A reviewed decision may
use those choices in the first runtime slice. Failed reproduction or review
feedback must revise the candidates before that slice.

## PostgreSQL-backed Tonic snapshot/watch prototype

### Source and reproduction

The feasibility-only service is generated from
`backend/servers/control-plane/crates/workflow-postgres/proto/watch.proto` inside the
PostgreSQL feasibility crate. It is not added to the production descriptor set or
registered by a production listener. The same disposable PostgreSQL command runs
both integration tests:

```sh
scripts/ci/check-workflow-postgres-feasibility.sh
```

One retained successful run reported:

```text
tonic-watch-feasibility: atomic-boundary=pass notification-replay=pass cursor-expiry=pass slow-reader=pass snapshot-p50-ms=36.273 snapshot-p95-ms=38.637 snapshot-max-ms=76.335 commit-ack-to-client-watch-p50-ms=34.751 commit-ack-to-client-watch-p95-ms=38.020 commit-ack-to-client-watch-max-ms=57.408 samples=100
test tonic_snapshot_watch_feasibility_gate ... ok
```

Snapshot samples measure loopback client request to receipt. The update helper
captures a monotonic timestamp immediately after PostgreSQL acknowledges `COMMIT`;
watch samples end when the loopback Tonic client receives that durable update. The
first ten of 110 deterministic samples are discarded as warm-up. These client-visible
measurements are reported diagnostically and are not compared with the contract's
distinct commit-notification-to-server-queue proposal or promoted to an approved SLO.

### Observations

1. Snapshot state and its resume boundary were selected in one repeatable-read
   transaction. A transactionally held update was excluded from both the initial
   projection and boundary, then delivered exactly once after commit even though its
   pre-listener `NOTIFY` could be missed. During a separate 100-update race, every
   returned revision equalled durable rows at that snapshot's boundary.
2. A cursorless stream emitted exactly one snapshot first and then a strictly later
   durable update. Versioned cursors were conversation-scoped; cross-conversation
   use returned `INVALID_ARGUMENT` without resolving the other conversation.
3. An update committed with `NOTIFY` suppressed was recovered by periodic durable
   table replay. Notifications are wake-ups only and never carry authoritative
   state.
4. Atomically advancing the synthetic retained watermark past an already-open
   watch caused that active stream, as well as a new older-cursor request, to return
   `FAILED_PRECONDITION: WATCH_CURSOR_EXPIRED`. Cursorless recovery returned current
   state and a usable boundary after all rows through that boundary were pruned.
5. A real Tonic client with a 1-KiB HTTP/2 receive window stopped polling while 32
   large updates filled a one-item application buffer. Only that watcher closed with
   `RESOURCE_EXHAUSTED`; a concurrent drained watcher received every update, mutation
   continued, and reconnect replayed every row after the slow client's last accepted
   cursor.
6. One local run retained 100 warmed samples with snapshot p50/p95/max of
   36.27/38.64/76.34 ms and commit-acknowledgement-to-client-watch p50/p95/max of
   34.75/38.02/57.41 ms.

### Limitations

- The service and schema are isolated feasibility artifacts, not the production
  private-v1 API or authority schema. The snapshot carries only a synthetic title
  and revision rather than the complete bounded M1 projection.
- Retention is advanced explicitly by the test; no seven-day pruning scheduler or
  wall-clock boundary is proven.
- The prototype opens simple plaintext connections rather than exercising the
  runtime pool, TLS, failover, cancellation storm, or multi-node behavior.
- Slow-reader eviction uses a deliberately constrained loopback HTTP/2 window. It
  does not size production transport buffers or prove behavior across every
  intermediary.
- The candidate requests carry only `conversation_id`; they do not prove the full
  Workspace/Project/Conversation parent-scope validation required by private v1.
- Replay maps a PostgreSQL failure to `UNAVAILABLE`, but this run does not inject an
  outage after an active watch has started.
- The latency values come from one local `linux/arm64` Docker fixture. They are
  diagnostic measurements, not an approved SLO, production benchmark, or evidence
  for the other client-visible M1 distributions.
- Cursor encoding, heartbeat cadence, replay batch size, and error telemetry remain
  candidate implementation details. The migration version still lacks SQL checksum
  drift detection.

### Decision effect

This result supports repeatable-read snapshot/boundary selection, durable rows with
`LISTEN/NOTIFY` as a lossy wake-up, scoped cursor expiry, and per-watcher bounded
buffers as viable candidates. It implements the experiment requested by feasibility
step 5 at the retained implementation revision, but maintainer review is still
required before the gate is recorded as closed. It does not authorize listener
registration or runtime PostgreSQL ownership.

## Authenticated JetStream topology prototype

### Source and reproduction

- NATS image: `nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4`.
- Tool image: `natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295`.
- Disposable config and stream fixtures: `dev/feasibility/jetstream`.

```sh
scripts/ci/check-workflow-jetstream-feasibility.sh
```

The successful run reported:

```text
workflow-stream-bytes=536870912 isolated-headroom=536870912 coexistence-headroom=536870912
max-command-body=262144 broker-stored-delta=266287 margin=4143
exact-republish-sha256=31a1f9dea0169551092d05e8bf4a446228c8c3eb4c9b713c66adcb7fd53c89be
jetstream-feasibility: auth=pass acls=pass consumers=pass bounds=pass payload=pass overhead=pass restart-republish=pass discard-new=pass expiry=pass
```

### Observations

1. Anonymous connections were rejected. Static fixture identities represented the
   application, one retained executor instance, the read-only acceptance harness,
   and an administrator.
2. Cross-role report/decision/command publication, direct executor subscription,
   and application stream administration were denied. Stream message deletion and
   purge were denied by the stream configuration.
3. Administrator-provisioned durable pull consumers let only the intended workload
   fetch and ACK command, report, per-executor decision, and wildcard acceptance
   deliveries through their narrow JetStream API/inbox permissions.
4. All three file-backed streams rendered and ran with `discard=new`, one replica,
   seven-day age, two-minute duplicate window, 260-KiB broker message maximum, and
   the contracted count/byte limits. Their 512-MiB sum leaves 512 MiB under the
   isolated 1-GiB budget. Adding the 512-MiB lifecycle stream under a 1.5-GiB
   coexistence budget also leaves 512 MiB.
5. A 256-KiB command body plus a near-4-KiB header was accepted. NATS reported a
   266,287-byte stored delta: 4,143 bytes over the body and below the contract's
   additional 8-KiB header/storage allowance.
6. After a broker stop/restart with retained file storage, republising the same
   exact bytes and `Nats-Msg-Id` produced a duplicate ACK at the original sequence.
   The fetched payload SHA-256 still matched the publisher file.
7. Scaled streams demonstrated rejection of a new record while retaining the old
   record under `discard=new`, and age expiry after a shortened fixture horizon.

### Limitations

The ACL identities use static plaintext fixture passwords inside a disposable
Docker network. Runtime credentials must use retained per-instance secret material;
the executor fixture does not prove Kubernetes Secret isolation or retirement.
The test validates the exact production limits but scales down stream-full and age
expiry execution, rather than writing the full 512-MiB reservation or waiting seven
days on every run. The coexistence result validates capacity arithmetic but does not
run the legacy lifecycle workload concurrently. Disk-full, long-outage expiry,
exact-byte publisher recovery after the duplicate window, and measured full-profile
filesystem headroom remain open before this gate can be approved.

### Decision effect

The image digests, three-stream separation, 327,680-byte server payload, narrow
workload ACL shape, 8-KiB combined header/storage reservation, and isolated/coexistence
budgets are viable candidates. The measured 4,143-byte overhead does not require an
increase to the contract's per-record margin. Approval still depends on the full
capacity/coexistence and credential-isolation evidence named above.

## Executor ledger and signing prototype

### Reproduction

```sh
cargo test --locked -p ayni-orchestrator-events workflow
cargo test --locked -p ayni-orchestrator-scripted-executor
pnpm --filter @ayni-orchestrator/grpc-bindings check
pnpm --filter @ayni-orchestrator/web exec node --import tsx ../../clients/typescript/tests/verification.test.ts
```

The retained successful run passed seven workflow event/signing tests and eight
executor-ledger tests. Rust and TypeScript independently reconstructed the canonical
sequence-1 report bytes and SHA-256 digest in
`proto/testdata/workflow/report-signing.json`, then verified the same Ed25519
signature. The signed input explicitly binds the event type, source, event ID,
assignment, executor generation, report sequence, command causation ID, and exact
deterministic protobuf bytes with the signature omitted.

The SQLite/WAL ledger prototype demonstrated:

- atomic retention of exact command and signed claim bytes before command ACK;
- restart with the same instance ID and signing seed, with execution still blocked
  before an accepted application decision;
- decoded application decisions bound to the retained report digest, workflow scope,
  assignment, generation, target instance, and accepted claimant before execution;
- equal command/report/decision replay after exact-byte cleanup, plus changed-byte
  conflict behavior;
- two retained executor instances receiving immutable winning and losing decisions,
  with only the winner able to create a terminal report;
- stale-generation rejection, fencing of a previously accepted older generation,
  and rejection of a late stale acceptance decision;
- terminal report validation against the retained execution binding and context
  revision;
- terminal report survival across reopen while its decision is lost;
- atomic ledger-full rejection; and
- report-byte cleanup only after a durable decision, followed by decision-byte
  cleanup only after its consumer ACK.

Tampering with signed binding or causation fields failed strict Ed25519 verification,
and a known small-order public key was rejected before ownership could bind. The
transport time and trace reference are required envelope fields but are not part
of the explicitly frozen signing input. The prototype uses pinned `rusqlite 0.40.2`
and the repository's existing `ed25519-dalek 2.2.0`.

### Limitations

This is an in-process state-machine prototype, not a JetStream-connected executor
process. Its fixture schema has no versioned upgrade path; runtime ledger migrations
must explicitly reject or migrate older layouts. The canonical workflow CloudEvents
require an RFC 3339 produced time and canonical W3C `traceparent`, bind complete
workflow scope and causal identity, and reject free-form failure diagnostics in
favor of closed safe-reason allowlists. Their generated event messages are an
intentional additive v1 compatibility surface in the events library so Rust/TypeScript
exercise one schema; no legacy event-type registration or runtime publisher/consumer
is required because this is the clean replacement protocol. Reopening the database
exercises durable restart boundaries but does not yet inject process kills at every
fsync/ACK boundary. The prototype stores
fixture signing seed bytes in SQLite; runtime file permissions, Secret injection,
and at-rest key handling are not approved by this evidence. Quarantine capacity,
actual NATS redelivery, per-instance credential/consumer retirement, ledger disk-full,
and old/new process overlap remain to be exercised in the integrated gate. The
fixture proves Rust/TypeScript canonicalization; another implementation language is
not yet a runtime dependency.

### Decision effect

The evidence supports SQLite WAL as the candidate bounded retained executor ledger,
Ed25519 over a domain-separated SHA-256 digest, and the checked-in protobuf fixture
as the candidate signing contract. These choices remain pending maintainer review
and integrated crash/transport evidence.

## Private-v1 reset and listener-separation prototype

### Source and reproduction

The complete candidate overlay, listener model, and retained consumer inventory live
under `dev/feasibility/private-v1/`. They are deliberately excluded from canonical
`proto/`, Rust descriptors, checked-in TypeScript bindings, and production listeners.

```sh
scripts/ci/check-workflow-private-v1-feasibility.sh
```

The successful result was:

```text
running 2 tests
test tests::custom_listener_has_no_workflow_service_registration ... ok
test tests::trusted_listener_has_only_the_postgres_owned_m1_services ... ok

private-v1-feasibility: schema=pass old-baseline-break=pass refreshed-baseline=pass post-reset-file-policy=pass generation=deterministic inventory=pass trusted-postgres-owner=pass custom-listener-omission=pass
```

### Observations

1. The isolated overlay freezes the two contracted service names and only the seven
   M1 RPCs: two hierarchy commands and five conversation operations. Workspace and
   Project keep approved field identities and reserve removed numbers and names.
2. The approved candidate makes snapshots self-contained with title and creation
   time, replaces full terminal Turns with a compact safe summary, uses one derived
   presentation-state enum for activity/update views, uses typed product vocabularies,
   and keeps safe diagnostic reason/detail values as bounded strings. Heartbeats are
   cursorless and separate from workflow progress.
3. Overlaying the candidate on the current module fails the old `FILE` baseline as
   expected. The exact refreshed overlay passes; deleting `SendMessage` afterward is
   rejected, demonstrating that normal `FILE` enforcement resumes after the reset.
4. Two complete TypeScript generations of the overlaid module were byte-for-byte
   identical. Rust generation remains build-time from the same canonical proto tree,
   so the eventual cutover must regenerate Rust descriptors and TypeScript from one
   revision.
5. The retained search found 63 handwritten Rust, BFF, web, and shell consumer files.
   File-set drift fails the script; generated TypeScript outputs are documented
   separately because generation uses `clean: true`.
6. The type-level listener model admits only a sealed PostgreSQL workflow owner on
   the trusted listener. Compile-fail checks reject a SQLite owner and reject any
   workflow-registration call on the custom-client listener.

### Limitations

- This is an isolated overlay and construction proof. Production still serves the
  old SQLite-backed conversation API, and the current custom listener still registers
  the old `ProjectService`; no authority switch is claimed.
- Temporary TypeScript generation is proven deterministic but is not checked in or
  compiled with the BFF/web consumers. The actual cutover must regenerate both client
  families, remove/adapt all inventoried consumers, and run their complete suites.
- The listener model proves a structural pattern, not the deployed server graph. The
  replacement must use equivalent separated constructors and transport tests that
  require both workflow service names to be unimplemented on the custom listener.
- `proto/BASELINE`, release documentation, and CI's one-time transition allowance are
  intentionally unchanged. They move only in the coordinated reset, after which the
  ordinary baseline guard must be restored.
- The name-based inventory can miss a consumer that avoids every searched symbol;
  cutover review must combine it with generated import and route ownership review.
- Results are retained at the implementation revision above; maintainer review is
  still required before this feasibility gate is recorded as closed.

### Decision effect

The approved candidate numbering, compact terminal projection, enum/string boundary,
one-time baseline procedure, known-consumer inventory, and separated-listener type
shape are viable inputs to the authority-switch change. This proof does not itself
change compatibility, ownership, registration, or client behavior.

## Validation retention policy

Routine pull-request validation compiles and tests the workspace, checks canonical
protobuf generation and compatibility, and validates documentation. The three
`check-workflow-*-feasibility.sh` commands are retained review gates rather than
automatic checks on every unrelated pull request: they pull pinned disposable images,
exercise host Docker networking and storage, or generate an isolated replacement API.
A review-ready evidence change reruns all three against its immutable implementation
revision and records the exact commands, image digests, observations, and limitations
here. Moving these gates into recurring hosted CI is a separate operational choice;
a missing hosted run is not represented as a passed result.

## Remaining gate ledger

| Gate | Evidence status |
| --- | --- |
| Current SQLite Orchestrator sandbox baseline | Open; must be reproduced safely or explicitly waived. |
| PostgreSQL client/migration feasibility | Evidence above; pending maintainer review. |
| Authenticated JetStream topology and capacity | Auth/ACL/bounds/overhead/restart evidence above; full-capacity, coexistence load, and credential isolation remain open. |
| Executor ledger, signing, and claim handshake | Local ledger/signing evidence above; integrated JetStream crash and retirement cases remain open. |
| PostgreSQL-backed Tonic snapshot/watch | Isolated implementation and local results are retained at the implementation revision above; maintainer review remains open. Production parent scoping, projection, retention scheduling, and listener integration remain out of scope. |
| Private-v1 reset and known-client regeneration | Complete isolated overlay, reset/regeneration procedure, 63-file handwritten inventory, and listener-separation model are retained at the implementation revision above; maintainer review remains open. Canonical baseline reset, generated-client cutover, production owner removal, and real listener tests belong to the coordinated authority switch. |
