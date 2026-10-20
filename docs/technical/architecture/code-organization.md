# Code organization

The Slice 1 workflow path has three production ownership boundaries.

| Path | Responsibility |
| --- | --- |
| `proto/ayni/orchestrator/v1` | Canonical private-v1 commands, receipts, workflow projections, errors, snapshots, and history |
| `backend/servers/control-plane/crates/grpc-contract` | Generated Rust API and transport-level request validation |
| `backend/servers/control-plane/crates/workflow-postgres` | Forward migrations, authoritative transactions, idempotency, snapshots, history, and exact outbox bytes |
| `backend/servers/control-plane` | Configuration, secret-file loading, HTTP probes, trusted-loopback gRPC mapping, and process lifecycle |
| `deploy/charts/ayni-orchestrator` | Reusable control-plane chart consuming administrator-owned database Secrets |
| `dev/charts/ayni-orchestrator-local` | Docker Desktop PostgreSQL 17.6 StatefulSet, role bootstrap, retained PVC, and reusable-chart composition |
| `clients/typescript` | Generated canonical TypeScript bindings and generation checks |

`workflow-postgres` does not depend on the gRPC contract. The control plane maps persistence models and safe errors into the canonical transport. This keeps database transaction rules independent of listener policy and avoids making generated transport types authoritative state.

The migration role exists only for startup schema work. `WorkflowStore` always receives the runtime-role URL. Configuration reads both URLs from mounted Secret files and does not accept database credentials in command-line arguments or ConfigMaps.

Future-slice feasibility code is explicitly named and isolated. It must not be registered in the Slice 1 listener or deployed by the local chart.
