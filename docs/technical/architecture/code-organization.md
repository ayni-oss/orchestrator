# Code organization

The Slice 2 workflow path separates transport contracts, PostgreSQL authority, broker I/O, and executor ownership.

| Path | Responsibility |
| --- | --- |
| `proto/ayni/orchestrator/v1` | Canonical private-v1 commands, receipts, projections, snapshots, history, and future watch frames |
| `proto/ayni/orchestrator/events/v1` | Exact command, signed report, and targeted decision payloads |
| `backend/libs/events` | Bounded CloudEvent encoding, identity/binding validation, signing, and strict verification |
| `backend/servers/control-plane/crates/grpc-contract` | Generated Rust API and transport-level request validation |
| `backend/servers/control-plane/crates/workflow-postgres` | Forward migrations, authoritative transactions, report decisions, FIFO promotion, projections, and exact outbox bytes |
| `backend/servers/control-plane` | Secret-file configuration, HTTP probes, loopback gRPC, outbox publication, report consumption, and process lifecycle |
| `backend/workers/scripted-executor` | Retained SQLite ledger, claim fencing, targeted-decision handling, and deterministic response execution |
| `deploy/charts/ayni-orchestrator` | Reusable control-plane/executor workloads consuming administrator-owned Secrets |
| `dev/charts/ayni-orchestrator-local` | Docker Desktop PostgreSQL, authenticated three-stream JetStream, retained PVCs, and reusable-chart composition |
| `clients/typescript` | Generated canonical TypeScript bindings and generation checks |

`workflow-postgres` does not depend on gRPC or a NATS client. The control plane maps persistence models into the canonical API and moves exact persisted bytes across the broker. The executor never receives a PostgreSQL credential. Its ledger library is transport-independent; the binary applies JetStream ACK ordering around ledger commits.

Migration, runtime, application-broker, executor-broker/signing, acceptance, and administrative identities remain distinct. Configuration loads long-lived credentials from mounted files and does not accept them in command-line arguments or ConfigMaps.

Slice 3 work—quarantine, complete fault injection, retention cleanup, metrics, and notification-backed watches—must extend these retained ownership boundaries rather than introduce a second authority.
