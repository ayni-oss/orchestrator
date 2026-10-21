# Ayni Orchestrator Helm chart

This reusable chart deploys the M1 Slice 2 control plane and one retained scripted executor. PostgreSQL remains the workflow authority and authenticated JetStream provides durable transport. The chart does not install either dependency or create credentials.

## Required Secrets

Create four distinct immutable, administrator-owned Secrets before installing:

- `workflowPostgres.migratorSecretName`: a `url` key for the migration-owner role;
- `workflowPostgres.runtimeSecretName`: a `url` key for the least-privilege application role;
- `workflowJetstream.applicationSecretName`: a `password` key for the application broker identity; and
- `workflowJetstream.executorSecretName`: `password`, `instance-id`, and `signing-seed-hex` keys owned only by the scripted executor.

`workflowPostgres.runtimeRoleName` must name the role in the runtime URL. All Secret names must differ. Database URLs, broker passwords, and the executor signing seed never enter ConfigMaps or command arguments. The migration init container mounts only the migration URL, the serving container mounts the runtime URL plus its broker password, and the executor mounts only its own broker/signing Secret and retained ledger.

The executor PVC retains exact commands and signed reports across restarts. The chart marks that claim with `helm.sh/resource-policy: keep`; deletion requires an explicit operator action after unresolved decisions are repaired or the disposable environment is intentionally reset.

## Deployment boundary

Slice 2 supports an isolated trusted local environment. PostgreSQL and NATS transport are plaintext only within that boundary. Do not use shared or production credentials or traffic until verified TLS and the later operational acceptance work are complete.

## Listeners

- HTTP health and readiness are exposed through the `control-plane` Service.
- Canonical private-v1 gRPC binds to pod loopback only. Use an explicit pod port-forward or the in-pod diagnostic.
- The scripted executor has no network listener, database credential, or Kubernetes API token.
- No custom-client workflow listener is installed.

See `values.yaml` for the supported values surface.
