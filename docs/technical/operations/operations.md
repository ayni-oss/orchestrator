# Slice 1 operations

This runbook covers the PostgreSQL-authoritative M1 Slice 1 control plane. It does not authorize shared-environment deployment or production data migration.

## Startup sequence

1. The migration init container reads only the migration PostgreSQL URL from its mounted Secret file.
2. It connects with the migration role and acquires the fixed transaction-scoped advisory lock.
3. It validates contiguous migration history and retained checksums.
4. It applies forward migration 1 if the database is empty.
5. It validates the exact catalog manifest, removes blanket default grants, and grants the configured runtime role explicit application privileges with read-only migration history.
6. The serving container reads only the runtime URL from its separate Secret file.
7. It repeats version, catalog-manifest, and least-privilege validation through the runtime role.
8. It binds HTTP and trusted-loopback gRPC listeners.

Any failure exits the process. Kubernetes restarts it; readiness never becomes successful while PostgreSQL or the schema is unavailable.

## Probes

`/healthz` means the process can answer HTTP. It intentionally does not depend on PostgreSQL.

`/readyz` opens a runtime-role connection and validates migration version, checksum, the deterministic PostgreSQL catalog manifest, and exact runtime privileges. A failure returns HTTP 503 without exposing a database error or credential.

Use both signals. Liveness should not restart a healthy process only because PostgreSQL is temporarily unavailable; readiness removes it from service.

## Schema changes

Migrations are repository-owned forward SQL. Each released migration needs:

- a monotonically increasing version;
- a stable description and retained SHA-256 checksum;
- transaction-safe DDL;
- concurrent-start evidence under the advisory lock;
- explicit runtime grants for newly created application tables, sequences, and functions;
- read-only runtime access to migration history with no schema-creation authority;
- an updated catalog-manifest checksum and malformed-schema/privilege failure tests.

Never edit an applied migration. A checksum mismatch, gap, unknown future version, or missing object is an operator-visible startup failure. Repair from a trusted matching backup or roll forward with a reviewed migration; do not rewrite migration history.

## Credentials and roles

The reusable chart never creates credentials. Administrators provide two differently named immutable Secrets containing a `url` key and configure `workflowPostgres.runtimeRoleName` to match the runtime URL:

- the migration owner, which can create schema objects;
- the runtime role, which has only the required table/column mutations, reads but cannot mutate migration history, and cannot create schemas or temporary objects, own workflow objects, inherit roles, or delegate grants.

The database administrator Secret belongs only to the local PostgreSQL chart and is not mounted into the control plane. URLs are files, not ConfigMap values, environment variables, logs, command-line arguments, API fields, or diagnostic output.

Immutable Secret rotation uses new Secret names followed by a chart update and rollout. For the disposable local environment, the supported reset is `dev/ctl purge <exact-namespace>` followed by deploy. Do not patch an immutable credential in place.

## Backup and restore

The local StatefulSet claim is retained across Helm uninstall. Namespace deletion removes it.

For a backup, quiesce workflow writes, use PostgreSQL-native backup tooling, and retain the migration table with application relations. A restore is acceptable only when migration version and checksum match the running binary. Start one control-plane replica first and wait for readiness before admitting traffic.

This clean-slate slice does not import SQLite state. SQLite files and compatibility-drain output are not valid PostgreSQL backups.

## Failure handling

| Signal | Interpretation | Action |
| --- | --- | --- |
| `/healthz` fails | Process or pod is unavailable | Inspect pod status and safe redacted logs |
| `/healthz` passes, `/readyz` fails | PostgreSQL connection, permission, migration, checksum, or schema failure | Check database readiness and migration history; do not bypass validation |
| `IDEMPOTENCY_CONFLICT` | Same scoped key was used with changed bytes | Reconcile the caller bug; use the retained original receipt for the original request |
| `RESOURCE_EXHAUSTED` | The 100-turn nonterminal bound was reached | Wait for a later slice to complete turns or reset the disposable test namespace |
| `HISTORY_SNAPSHOT_EXPIRED` | Stable pagination boundary exceeded its lifetime | Fetch a new conversation snapshot and restart pagination |
| `WatchConversation` is `UNIMPLEMENTED` | Expected Slice 1 behavior | Use snapshot/history; watch replay enters Slice 3 |
| Future schema version | Binary is older than retained state | Deploy a compatible newer binary; never downgrade against the database |

Errors and logs expose only stable safe reasons. User content, database URLs, passwords, exact command bytes, and raw database diagnostics must not be logged.

## Rollback boundary

Before the first PostgreSQL command is accepted, revert the chart and binary. After admission, roll forward. Because this is a disposable replacement environment with no adopted data, a deliberate namespace reset is allowed; silently deleting or importing retained SQLite data is not.
