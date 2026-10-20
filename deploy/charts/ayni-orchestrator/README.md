# Ayni Orchestrator Helm chart

This reusable chart deploys the Slice 1 control plane. PostgreSQL is the sole workflow authority; the chart does not install a database or create credentials.

## Required Secrets

Create two immutable, administrator-owned Secrets before installing the chart:

- `workflowPostgres.migratorSecretName`: a `url` key for the migration-owner role.
- `workflowPostgres.runtimeSecretName`: a `url` key for the least-privilege application role.
- `workflowPostgres.runtimeRoleName`: the runtime role named by that URL. It must be distinct from the migration owner so the migration runner can grant only application-table DML and read-only migration history.

The two Secret names must differ. Database URLs never enter a ConfigMap or command-line argument. A startup init container mounts only the migration URL, applies and validates the schema, removes unsafe default grants, and grants the named runtime role. The serving container mounts only the runtime URL and validates the exact catalog manifest and runtime privileges before binding listeners.

## Deployment boundary

This Slice 1 chart is a contributor-preview deployment for an isolated trusted environment. The current PostgreSQL connector is plaintext and intentionally supports Docker Desktop acceptance only. Do not use it for shared or production credentials or traffic; verified TLS configuration is required before that deployment boundary can expand.

## Listeners

- HTTP health and readiness are exposed through the `control-plane` Service.
- Canonical private-v1 gRPC binds to pod loopback only. Use an explicit pod port-forward for trusted diagnostics.
- No custom-client workflow listener is installed in Slice 1.

See `values.yaml` for the complete supported values surface.
