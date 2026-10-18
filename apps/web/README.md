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

Use **Ayni Orchestrator** for the application name. The header, favicon,
documentation site, and README use the Orchestrator mark. `AyniIcon` defaults to
the CLI mark for the Ayni configuration entry and repository runtime badges;
`variant="orchestrator"` selects the application mark. Inline icons follow the
current text color, and favicons adapt to the browser's light or dark preference.
The `Ayni` configuration section refers specifically to CLI runtime certificates.

## UI transition

The workspace Projects view starts at `/`; `?workspace=<id>` selects its scope.
It uses real workspace/project data and the existing validated creation flow.
Project details temporarily link into `/_old`. Workspace settings live at
`/workspaces/<id>/settings` inside the shared workspace shell; the header menu,
and sidebar link there. Legacy settings bookmarks redirect
to this canonical route.

The previous application is preserved in `src/routes/_old/`, including its layout,
server loads/actions, update streams, and GitHub manifest setup callbacks. Its
internal URLs use the `/_old` prefix. `/healthz`, static assets, and the global
security hooks remain at their existing locations. Old root-level page URLs are
not aliases; those paths are reserved for the new interface.

During migration, shared components and server helpers still live in `src/lib`.
Remove legacy routes only after their behavior has been replaced; trace their
imports before deleting shared modules. The new creation form currently shares
`create-project.svelte` with the previous workspace page. Repository data is not
copied or reset by this UI migration.
