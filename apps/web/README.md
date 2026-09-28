# SvelteKit browser server

This private adapter-node application hosts the same-origin browser boundary. A
server load and form action demonstrate the BFF integration, and
`/tasks/{task_id}/updates` relays bounded resumable gRPC updates as server-sent
events. The complete operator interface is tracked separately.

The application must run beside the control plane and use
`ORCHESTRATOR_GRPC_ADDRESS=127.0.0.1:50051`. It receives no Kubernetes
service-account token or credential mounts. Native gRPC bindings and the BFF may
only be imported by server modules. `pnpm frontend:check` builds and scans the
client bundle to enforce that boundary.
