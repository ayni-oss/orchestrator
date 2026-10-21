# Code organization

The durable workflow path separates transport contracts, PostgreSQL authority, broker I/O, and executor ownership.

| Path | Responsibility |
| --- | --- |
| `proto/ayni/orchestrator/v1` | Canonical private-v1 commands, receipts, projections, snapshots, history, watches, configuration and session commands |
| `proto/ayni/orchestrator/events/v1` | Exact command, signed report, and targeted decision payloads |
| `backend/libs/events` | Bounded CloudEvent encoding, identity/binding validation, signing, and strict verification |
| `backend/servers/control-plane/crates/grpc-contract` | Generated Rust API and transport-level request validation |
| `backend/servers/control-plane/crates/workflow-postgres` | Forward migrations, immutable configuration/turn inputs, session generations, authoritative transactions, report decisions, FIFO promotion, projections, immutable plan review and exact outbox bytes |
| `backend/servers/control-plane` | Secret-file configuration, HTTP probes, loopback gRPC, outbox publication, report consumption, and process lifecycle |
| `backend/workers/scripted-executor` | Retained broker authority ledger, claim fencing, targeted decisions, scoped Kubernetes session provisioning and deterministic session workers |
| `deploy/charts/ayni-orchestrator` | Reusable control-plane/executor workloads consuming administrator-owned Secrets |
| `dev/charts/ayni-orchestrator-local` | Docker Desktop PostgreSQL, authenticated three-stream JetStream, retained PVCs, and reusable-chart composition |
| `clients/typescript` | Generated canonical TypeScript bindings and generation checks |

`workflow-postgres` does not depend on gRPC or a NATS client. The control plane maps persistence models into the canonical API and moves exact persisted bytes across the broker. The executor never receives a PostgreSQL credential. Its ledger library is transport-independent; the binary applies JetStream ACK ordering around ledger commits.

Migration, runtime, application-broker, executor-broker/signing, acceptance, and administrative identities remain distinct. Configuration loads long-lived credentials from mounted files and does not accept them in command-line arguments or ConfigMaps.

Quarantine, fault injection, retention cleanup, metrics and notification-backed
watches extend these retained ownership boundaries. The [conversation execution
contract](conversation-execution.md) defines M2 provider and tool safety; the
[provider request contract](provider-requests.md) owns future paid-call recovery.
