# Durable workflow operations

This runbook covers the PostgreSQL-authoritative M1 Slice 3 control plane, isolated authenticated JetStream profile, retained scripted supervisor and
conversation-scoped session workers. It does not authorize shared-environment deployment, production credentials, or production data migration.

## Startup sequence

1. The migration init container reads only its PostgreSQL URL, takes the transaction-scoped advisory lock, validates retained history/checksums, applies migrations through ordinal 9, installs runtime grants, and validates the catalog manifest.
2. PostgreSQL, JetStream, and executor-ledger retained volumes are mounted without deleting existing state.
3. NATS starts with four password identities and no anonymous access. The administrative hook creates or updates the three exact streams and four durable consumers.
4. The control plane validates the runtime database role, binds HTTP and loopback gRPC, loads only the application broker password, and resumes pending outbox/report acceptance.
5. The executor validates its retained instance/key identity, binds the pre-provisioned command and targeted-decision consumers, reconciles decision ACK floors, republishes pending reports, and resumes accepted commands.

Broker unavailability does not make PostgreSQL readiness false. Both broker clients retry without discarding retained owner state.

## Probes and checks

`/healthz` reports process liveness without consulting dependencies. `/readyz` validates PostgreSQL migration version, checksums, catalog manifest, and exact runtime privileges. Database failure returns HTTP 503 without exposing a credential or raw diagnostic. A full or unsupported-version report quarantine also returns a named readiness failure. `/metrics` reports durable queue/outbox/quarantine pressure and process timing/counters using safe identities.

Use the local operational checks:

```sh
dev/ctl check
dev/ctl scenario
```

`check` verifies all StatefulSets/Deployments, migrations through ordinal 9, exact stream policy/capacity, named consumers, and executor-ledger initialization. `scenario` proves one private-v1 message reaches one accepted scripted response.

## Configuration and session recovery

The current configuration pointer is mutable; each configuration revision and
admitted turn input is immutable. `UpdateAgentConfiguration` uses expected
revision and scoped idempotency. Resolve stale-edit conflicts by reading the
current configuration, not by changing the command key and overwriting blindly.
`GetTurnInputs` and `GetTurnContextInputs` expose bounded frozen input evidence.
Admission reserves the 10,000-reference context limit conservatively, including
possible predecessor responses. Capacity is rejected before message admission
rather than while accepting a predecessor result.

`ListConversationSessions` lists retained generations for one conversation.
`ReplaceConversationSession` requires the expected generation and original
idempotency key. The listing's advisory `can_replace` flag becomes true only
for an idle or expired generation with no active work or unresolved outbox handoff. The mutation
rechecks that condition under the conversation lock before fencing it.

The next eligible turn advances the generation. An active accepted execution recovers through its existing authority
ledger; replacement is not an implicit retry.

Session result ledgers and their scoped credentials join the retained-state
boundary. Preserve them with PostgreSQL, JetStream and the supervisor ledger.
A session worker has no database credential, broker credential or Kubernetes API
token. The trusted supervisor alone provisions session workloads. Provider
credentials remain disabled in the scripted scope.

The dedicated session namespace is `<workflow-namespace>-sessions`, labelled
with its parent namespace and retained by Helm. Each generation has an immutable
bearer Secret and its own 512 MiB PVC. The supervisor has get/create/delete Pod
rights and get/create Secret/PVC rights only in that namespace. Workers have no
Kubernetes API token. The NetworkPolicy declares supervisor-only ingress and no
worker egress; cluster network enforcement must be verified separately.

Worker ledgers retain exact requests and results until the application decision
handoff. Resolved entries release request contents and retain digest/result
tombstones for eight days. Execution prunes expired resolved tombstones and
fails closed at 50,000 rows or 256 MiB active retained bytes. Idle exit requires
no unacknowledged result. Restart adopts the same generation's PVC; a turn after
expiry/replacement receives a new generation and separate PVC.

## Schema and stream changes

Every forward migration requires a monotonically increasing installation ordinal, stable checksum, transaction-safe DDL, advisory-lock concurrency evidence, explicit runtime grants, an updated manifest checksum, and malformed-schema/privilege failure tests. Never edit an applied migration.

All owned interfaces and retained formats stay v1 until alpha merges. SQL
migration ordinals continue to advance within that contract.

The broker profile retains the M1 bounds. A reviewed change must preserve independent command/report/decision capacity, `discard=new`, oldest-record survival, seven-day age, denied delete/purge, the server/PVC headroom calculation, and least-subject ACLs. Provision consumers before new producers.

## Credentials and roles

The reusable chart consumes four differently named immutable Secrets:

- migration PostgreSQL URL;
- runtime PostgreSQL URL;
- application NATS password; and
- executor NATS password, retained instance ID, and Ed25519 seed.

The local chart additionally owns separate NATS administrative and acceptance-harness passwords. The database administrator belongs only to PostgreSQL. Secret contents never enter ConfigMaps, command arguments, logs, events, gRPC, or JetStream payloads.

Rotate immutable credentials with new Secret names and an ordered workload rollout. Do not retire the executor credential, key, consumer, or ledger while a targeted decision is unresolved. The disposable local reset is `dev/ctl purge <exact-namespace>` followed by deploy; do not patch an immutable Secret in place.

## Backup and restore

The session namespace has Helm keep policy. `dev/ctl cleanup` stops the supervisor
before removing owned session Pods and preserves all workflow/session PVCs and
immutable Secrets. Exact-parent `dev/ctl purge` deletes both ownership-labelled
namespaces; it permanently removes those stores.

A consistent backup quiesces admission and preserves PostgreSQL, JetStream,
the supervisor ledger and every unresolved session ledger with its scoped
credentials. PostgreSQL alone is insufficient while any command/report/decision handoff is unresolved. Restore only matching migration/checksum and protocol revisions, start stateful dependencies first, then one control plane and the retained executor, and run `dev/ctl check` before admitting new work.

SQLite workflow files from the superseded implementation are not backups. The executor ledger is a separate current protocol store and must not be imported into PostgreSQL.

## Failure handling

| Signal | Interpretation | Action |
| --- | --- | --- |
| `/healthz` fails | Process or Pod unavailable | Inspect Pod status and safe redacted logs |
| `/readyz` fails | PostgreSQL connection, role, migration, checksum, or schema failure | Repair the database boundary; never bypass validation |
| Outbox remains `pending`/`stored` | Broker unavailable/full or no accepted owner | Restore broker/executor; preserve exact PostgreSQL bytes |
| Turn remains `processing` | Accepted retained executor has not produced an accepted terminal report | Restore that executor and its ledger; M1 does not reassign an accepted owner |
| Executor transport retries | Stream/consumer/ACL unavailable | Repair the pre-provisioned broker profile; do not grant admin rights to the workload |
| `IDEMPOTENCY_CONFLICT` | Scoped command key reused with changed bytes | Reconcile the caller and retain the original receipt |
| `RESOURCE_EXHAUSTED` | FIFO or retained-capacity bound reached | Restore capacity without deleting accepted owner state |
| `HISTORY_SNAPSHOT_EXPIRED` | Stable pagination boundary expired | Fetch a fresh snapshot and restart pagination |
| `WATCH_CURSOR_EXPIRED` | Cursor predates the seven-day replay watermark | Resume without a cursor to receive an authoritative snapshot |
| Watch `RESOURCE_EXHAUSTED` | Reader exhausted its bounded delivery buffer | Reconnect with the last consumed cursor |
| `workflow_report_quarantine_full` | Report quarantine reached a count or byte limit | Restore compatible consumption; keep producing-side exact bytes |
| `workflow_report_quarantine_unsupported_version` | Retained report requires a newer consumer | Upgrade the consumer before the producer and allow exact-byte replay |
| Future schema version | Binary older than retained state | Deploy a compatible newer binary; never downgrade against retained stores |

Logs expose only stable IDs and safe reason codes. User content, credentials, exact command/report bytes, signatures, and raw database diagnostics must not be logged.

## Rollback boundary

Before the first PostgreSQL command, the chart and binaries can be reverted. After admission, roll forward while preserving all retained owners. Roll back to Slice 1 only after proving no command/report/decision handoff is unresolved. Otherwise repair forward or explicitly reset the isolated disposable namespace; never silently delete a PVC or translate data back into SQLite.
