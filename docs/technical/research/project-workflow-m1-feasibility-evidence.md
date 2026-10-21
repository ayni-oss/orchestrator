# Project workflow M1 feasibility evidence

Date: 2026-10-20. Status: all M1 pre-implementation gates are closed for
runtime slice 1. This bundle approves the bounded architecture and acceptance
criteria; it does not claim that the production M1 runtime has been implemented.

This document retains reproducible evidence for the gates in
[the M1 durable-foundation contract](project-workflow-m1-durable-foundation.md).

## Retained revision and scope

- Repository: `ayni-oss/orchestrator`.
- Executable proof and contract revision:
  `54469ec99b3534cd53883128aae304e5f65b48e5`.
- Rust: `rustc 1.94.0`; Cargo 1.94.0.
- PostgreSQL image:
  `postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3`.
- NATS image:
  `nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4`.
- NATS tool image:
  `natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295`.
- Exercised host: Docker Desktop on `linux/arm64`; the credential proof used the
  disposable `docker-desktop` Kubernetes context.

The maintained clean-slate decision explicitly waives reproduction, migration, or
compatibility with the prior SQLite workflow runtime. Prior schemas, retained data,
APIs, tests, behavior, and broker topology are learning inputs only. Runtime slice 1
must delete that workflow implementation and its tests while installing the
replacement boundary. No dual write, compatibility shim, old baseline deployment,
or shared-broker adaptation is part of M1.

## Approved decisions

The maintainer approved these choices on 2026-10-20:

| Area | Approved M1 decision |
| --- | --- |
| PostgreSQL | PostgreSQL 17.6, `tokio-postgres 0.7.18`, repository-owned forward SQL, and transaction-scoped advisory locking. |
| Snapshot/watch | Repeatable-read snapshot and boundary selection; durable-table replay; `LISTEN/NOTIFY` only as a lossy wake-up. |
| Private v1 | One intentional clean reset followed immediately by normal `FILE` compatibility enforcement; workflow services exist only on the trusted listener. |
| Broker | A new isolated authenticated JetStream profile; no legacy publisher or shared-broker coexistence work. |
| Credentials | Distinct least-privilege per-instance passwords in immutable Kubernetes Secrets. Broker configuration retains only one-way hashes. Plaintext transport is a Docker Desktop development exception; TLS is required before production secrets or traffic. |
| Executor identity | SQLite WAL retained ledger, Ed25519 report signatures, one retained ledger/PVC per accepted owner, and no executor-generation rotation after ownership acceptance. |
| Performance | Internal p95 limits: 50/100/100/250/100 ms for authority commit, outbox-to-broker storage ACK, report validation/decision commit, claim-decision round trip, and server snapshot. Client-visible p95 limits: 250/500/2,000 ms for send, watch propagation, and answer resumption. |

The approved performance values are runtime acceptance limits, not claims that every
production distribution exists in these prototypes. The watch prototype below
measures the distributions it owns; runtime slices must add the remaining warmed
100-sample checks before taking their respective authority.

## Reproduction

The JetStream and executor results below are historical gate evidence owned by the
pinned proof revision `54469ec99b3534cd53883128aae304e5f65b48e5`. Reproduce
those results from a clean checkout of that revision, where the isolated fixtures
and gate scripts remain intact. Runtime Slice 1 deliberately removes those local
broker/executor fixtures rather than carrying an unused or shared-broker profile.
They return with the isolated topology in runtime Slice 2.

At that revision, the active Slice 1 checkout retained and ran only its authority and
cutover gates:

```sh
scripts/ci/check-workflow-postgres-feasibility.sh
scripts/ci/check-workflow-private-v1-feasibility.sh
```

Both scripts use pinned dependencies, disposable resources, bounded waits, and
cleanup traps. They do not read production Secrets or external-provider
credentials.

## PostgreSQL authority and migration gate

The PostgreSQL script starts one disposable PostgreSQL 17.6 container and runs the
ignored integration tests in the feasibility crate. The retained result was:

```text
postgres-feasibility: advisory-lock=pass rollback=pass concurrent-migration=pass notification-replay=pass fifo-row-lock=pass exact-outbox=pass
test postgres_feasibility_gate ... ok
```

The proof established:

1. A separately held transaction-scoped advisory lock visibly blocked the migration
   backend on the exact lock key before DDL.
2. Eight concurrent migration callers completed while exactly one version row was
   retained.
3. Explicit rollback left no partial authority state.
4. Sixteen competing admissions allocated the contiguous FIFO range 1 through 16
   through row locking.
5. Exact outbox bytes, including NUL and non-UTF-8 octets, survived a fresh
   connection without transformation.
6. A notification lost while disconnected was recovered from the durable table;
   a later notification acted only as a live wake-up.

This closes selection of PostgreSQL 17.6, `tokio-postgres 0.7.18`, forward migration
SQL, advisory locking, row-lock admission, exact `bytea` outbox storage, and durable
replay. Production schema checksums, least-privilege runtime/migration roles, TLS,
pooling, failover, and calendar-time retention remain runtime implementation work.

## Snapshot/watch gate

The same script runs an isolated Tonic service against PostgreSQL. The latest retained
100-sample run, after ten warm-up samples, reported:

```text
tonic-watch-feasibility: atomic-boundary=pass notification-replay=pass cursor-expiry=pass slow-reader=pass snapshot-p50-ms=36.780 snapshot-p95-ms=44.922 snapshot-max-ms=68.237 commit-ack-to-client-watch-p50-ms=34.802 commit-ack-to-client-watch-p95-ms=37.251 commit-ack-to-client-watch-max-ms=62.473 samples=100
```

Both measured p95 values are below the approved 100-ms server snapshot limit and the
500-ms client-visible watch limit. The proof also established:

- one repeatable-read transaction selects projection state and its resume boundary;
- a concurrent committed update is excluded from both or included in both, then is
  delivered exactly once after the boundary;
- cursorless watch emits one snapshot before later durable updates;
- cursors are scoped to one conversation and old/pruned cursors fail with
  `FAILED_PRECONDITION`;
- suppressed notifications are recovered by durable replay;
- one slow reader is evicted with `RESOURCE_EXHAUSTED` without blocking mutation or
  another watcher; and
- reconnect resumes from the last accepted cursor without loss.

The prototype uses synthetic bounded state and explicit retention advancement.
Production parent-scope validation, complete projections, seven-day pruning,
transport sizing, TLS, outage injection, and listener registration remain runtime
acceptance work rather than open architecture decisions.

## Authenticated isolated JetStream gate

The topology proof retained this result:

```text
workflow-stream-bytes=536870912 isolated-headroom=536870912
max-command-body=262144 broker-stored-delta=266287 margin=4143
exact-republish-sha256=31a1f9dea0169551092d05e8bf4a446228c8c3eb4c9b713c66adcb7fd53c89be
jetstream-feasibility: auth=pass acls=pass consumers=pass bounds=pass payload=pass overhead=pass restart-republish=pass discard-new=pass expiry=pass
```

Anonymous access; cross-role publication; direct executor subscription; application
administration; and stream deletion/purge were denied. Administrator-created pull
consumers limited each role to its command, report, per-executor decision, or
read-only acceptance path. The three streams use file storage, one replica,
`discard=new`, seven-day age, two-minute duplicate window, 260-KiB message maximum,
and denied delete/purge.

A maximum 256-KiB command plus near-4-KiB headers was accepted. The 266,287-byte
stored delta leaves 4,143 bytes of measured overhead, below the reserved 8 KiB.
Broker restart retained exact bytes and the original duplicate ACK sequence. Scaled
fixtures independently proved age expiry and rejection of a new record without
removing the oldest record.

### Production-size capacity

The separate capacity script writes the isolated profile to each configured byte
limit. It accepts a full result only when the failed publish reports a broker
capacity error, retained usage is within one conservative record of the limit, the
successful publish count equals retained messages, and record 1 still has the
expected digest. The retained result was:

```text
capacity-stream=ORCHESTRATOR_WORKFLOW_COMMAND_V1 messages=1023 stored-bytes=268250081 configured-max-bytes=268435456 discard-new=pass oldest-retained=pass
capacity-stream=ORCHESTRATOR_WORKFLOW_REPORT_V1 messages=6130 stored-bytes=201321503 configured-max-bytes=201326592 discard-new=pass oldest-retained=pass
capacity-stream=ORCHESTRATOR_WORKFLOW_DECISION_V1 messages=4067 stored-bytes=67093344 configured-max-bytes=67108864 discard-new=pass oldest-retained=pass
capacity-profile=isolated stream-bytes=536664928 filesystem-bytes=549261312 server-headroom=537076896 pvc-headroom=1598222336
jetstream-capacity: isolated-full=pass discard-new=pass filesystem-headroom=pass
```

The measured profile leaves at least 512 MiB under both the 1-GiB logical server
budget and 2-GiB PVC budget. Disk-full behavior below that reserved headroom,
multi-node replication, production TLS, and long calendar-time age expiry remain
runtime/deployment concerns.

## Kubernetes credential-isolation gate

The credential script creates a disposable namespace and separate service accounts,
immutable Secrets, and hardened client Pods for administrator, application, two
executor instances, and acceptance identities. The broker ConfigMap is rejected if
it contains any fixture plaintext password; checked-in broker configuration contains
only bcrypt hashes. No client Pod mounts another identity's Secret, service-account
tokens are disabled, and namespace RBAC grants no Secret read authority.

The retained result was:

```text
denied=diagnostic-anonymous
denied=winner-reads-loser-decision
denied=loser-reads-winner-decision
denied=acceptance-publishes
retirement-blocked=targeted-decision-unacked
denied=retired-consumer
jetstream-credentials: immutable-secrets=pass broker-hashes=pass pod-isolation=pass rbac-denial=pass executor-isolation=pass retirement-order=pass
```

Each executor could consume only its targeted decision. Retirement remained blocked
while one decision was ACK-pending, then consumer deletion made further access fail.
This closes the local password lifecycle and isolation gate. Plaintext client
transport is confined to this disposable local environment; a production deployment
must add TLS before admitting any credential or traffic.

## Executor ledger, signing, and handoff gate

The event library now enforces separate 16-KiB ownership-claim and 32-KiB terminal
report limits on encode and decode. A valid signed claim padded to max-plus-one is
rejected. Strict Ed25519 verification rejects changed binding/causation fields and
known weak public keys.

The bounded SQLite/WAL ledger tests establish:

- command and signed claim bytes commit atomically before command ACK;
- the claim public key must equal the retained seed's derived public key;
- restart retains instance ID, signing seed, exact bytes, and the execution fence;
- terminal-report capacity is reserved when accepting a command/claim;
- real SQLite-full and logical-capacity failures roll back command and claim together;
- stale generations fail, an unaccepted generation may be replaced, and an accepted
  owner cannot rotate in M1;
- only the accepted split-brain claimant can execute;
- exact-byte duplicates remain valid after byte cleanup while changed bytes conflict;
- unresolved entries never prune; completed/rejected metadata prunes only after the
  fixed eight-day horizon; and
- retirement requires no unresolved execution and every targeted decision ACK.

The authenticated transport wrapper retained this result:

```text
executor-transport-feasibility: command-redelivery=pass split-brain=pass decision-redelivery=pass ack-reconciliation=pass long-outage-republish=pass retirement=pass
executor-feasibility: authenticated-transport=pass crash-redelivery=pass split-brain=pass ack-reconciliation=pass long-outage=pass retirement=pass
```

The integration drops a command delivery before ACK, reopens the winning ledger,
accepts competing claims, targets immutable winner/loser decisions, and proves only
the winner can execute. Before a decision ACK, the ledger stores the named consumer's
delivery sequence. After the broker confirms the ACK, the test closes the ledger and
reconciles that sequence against the consumer's contiguous ACK floor. A wrong
consumer and a lower floor fail closed. This removes the otherwise ambiguous crash
window between broker ACK and local retirement state.

The integration also shortens report age and duplicate windows, then republishes the
same terminal report identity and exact bytes after both horizons. Retirement succeeds
only after both claim and terminal decisions are confirmed and reconciled. Runtime
file permissions, fsync/storage-class qualification, production Secret projection,
and kill injection at additional process boundaries remain slice acceptance work.

## Private-v1 reset and listener gate

Before implementation, an isolated overlay under `dev/feasibility/private-v1/`
proved the one-time reset, deterministic generation, complete handwritten-consumer
inventory, trusted PostgreSQL owner, and custom-listener omission. That overlay and
inventory were deleted after the coordinated canonical cutover; preserving a second
schema source would now be drift.

The post-cutover gate reports:

```text
private-v1-cutover: canonical-surface=pass stale-consumers=absent trusted-loopback=pass custom-listener=absent
```

The canonical sources now freeze two service names and seven M1 RPCs, reserve removed
field names and numbers, and define compact terminal projections, typed vocabularies,
and cursorless heartbeats. TypeScript generation is checked against committed output.
Normal `FILE` compatibility resumes after the `workflow-m1-private-v1` baseline.
The old workflow owner, tests, BFF/web callers, and inventoried handwritten consumers
were removed with the cutover.

## Review closure

Independent read-only audit initially found four blocking proof defects:

1. no recoverable executor state for a crash after broker decision ACK;
2. ownership claims accepted up to the terminal report limit;
3. capacity rejection accepted arbitrary publish failures and checked only positive
   headroom; and
4. plaintext client passwords present in the broker ConfigMap.

Revision `13afc851f69c77e0696da70b7f17ca5114693509` closes all four with retained
consumer sequence/ACK-floor reconciliation, kind-specific claim bounds,
capacity-error/near-limit/512-MiB assertions, and bcrypt-only broker configuration.

A final audit found that the executor byte reservation omitted claim margin and a
future terminal record, terminal rejection had no completion path, report duplicate
checks ignored the command binding, and the private-v1 inventory used a partial
symbol list. Revision `60aef6cbbfc92755820d0d342f36ad0ee4bc5a08` closes those gaps with
fixed-size record/byte reservations, terminal rejection cleanup, bound duplicate
checks, a legacy-proto-derived 154-file inventory, and a refreshed Ayni environment
lock. Revision `54469ec99b3534cd53883128aae304e5f65b48e5` then preserves the authenticated
transport scenario while splitting it into bounded phase helpers; the pinned Rust
analyzer reports a maximum cyclomatic complexity of 19 in that file rather than the
previous failing value of 66. The focused and full validation commands below passed
after those corrections.

## Validation and gate ledger

In addition to the six gate scripts, the retained revision passed:

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets
pnpm docs:check
pnpm frontend:check
pnpm test:frontend
scripts/ci/check-proto.sh
PATH=/opt/homebrew/bin:$PATH scripts/ci/check-helm.sh
scripts/ci/check-github-security.sh
node --test dev/tests/nats-fault-proxy.test.mjs dev/tests/planner-stub.test.mjs
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s evals -p 'test_*.py'
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/operations -p 'test_*.py'
```

The Homebrew Bash prefix avoids the known empty-array behavior of macOS Bash 3.2 in
an existing development-secret test; the same Helm command succeeds under modern
Bash used by CI.

| Gate | Closed evidence |
| --- | --- |
| Historical SQLite sandbox baseline | Explicitly waived by the clean-slate decision; no compatibility, data migration, old tests, or shared topology is allowed. |
| Architecture and performance choices | Approved choices and p95 limits are recorded above and in the durable contract. |
| PostgreSQL client/migration | Advisory lock, rollback, concurrency, FIFO row lock, exact bytes, and replay pass. |
| Snapshot/watch | Atomic boundary, lossy-notification replay, expiry, slow-reader isolation, and warmed latency pass. |
| JetStream topology/capacity | Auth/ACLs, bounds, max payload, restart, exact republish, full production-size `discard=new`, oldest retention, and both 512-MiB headrooms pass. |
| Credential isolation/retirement | Immutable per-identity Secrets, bcrypt-only broker config, pod/RBAC isolation, per-executor decisions, and ACK-before-retirement pass. |
| Executor ownership/handoff | Strict signing and claim size, atomic ledger, capacity reservation, split brain, crash/redelivery, ACK-floor reconciliation, long-outage republish, and retirement pass. |
| Private-v1 reset/listeners | Reset procedure, regenerated baseline enforcement, deterministic generation, 154-file inventory, trusted PostgreSQL owner, and custom-listener omission pass. |

All pre-implementation gates are therefore closed. The remaining items in this
document are explicit runtime-slice acceptance obligations, not unresolved choices
or permission to restore the superseded architecture.
