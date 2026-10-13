# Ayni Orchestrator web application

This private adapter-node application provides the supported browser workflow
for repository readiness, development tasks, lifecycle actions, usage, and
retained evidence. Server loads and form actions call the server-only BFF;
`/tasks/{task_id}/updates` relays bounded resumable native-gRPC updates as
server-sent events.

The application runs beside the control plane with
`ORCHESTRATOR_GRPC_ADDRESS=127.0.0.1:50051`. It receives no Kubernetes
service-account token or credential mounts. Native gRPC bindings and the BFF may
only be imported by server modules. `pnpm frontend:check` type-checks, tests,
builds, and scans the browser bundle to enforce that boundary. Playwright tests
exercise the operator workflow against a deterministic native-gRPC fixture.

## Product identity

Use **Ayni Orchestrator** for the application name and the shared Ayni icon in
the header and favicon. `AyniIcon` uses the same geometry as the documentation
site and README logo; it follows the current text color. The `Ayni` configuration
section refers specifically to CLI runtime certificates.
