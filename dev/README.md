# Local Slice 1 stack

The local workflow is deliberately small: one Ayni control plane and one PostgreSQL 17.6 StatefulSet on Docker Desktop Kubernetes. PostgreSQL is the sole workflow authority. Browser, worker-runtime, JetStream, and custom-client workflow paths are not part of Slice 1.

## Prerequisites

- Docker Desktop Kubernetes with current context `docker-desktop`.
- Helm 4.1.1.
- `kubectl`, Docker, and OpenSSL.
- At least 8 GiB free on the host and Docker Desktop VM.

## Deploy

```sh
dev/ctl deploy
dev/ctl check --expect-empty
```

The first deploy creates three immutable Secrets without printing their values:

- `workflow-postgres-admin`, used only by the PostgreSQL container.
- `workflow-postgres-migrator`, mounted into the control plane for startup migration.
- `workflow-postgres-runtime`, used for serving requests after migration.

The chart puts credentials only in Secret-backed files or Secret references, never in ConfigMaps or command-line arguments.

## Diagnostics

The canonical private-v1 listener binds only to loopback inside the control-plane pod. Port-forward the pod explicitly before running the repository diagnostic client:

```sh
pod=$(kubectl -n ayni-orchestrator-dev get pod \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
  -o jsonpath='{.items[0].metadata.name}')
kubectl -n ayni-orchestrator-dev port-forward "pod/$pod" 50051:50051
```

HTTP health and readiness can be reached separately with:

```sh
kubectl -n ayni-orchestrator-dev port-forward service/control-plane 8080:8080
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

## Retention and teardown

`dev/ctl cleanup` uninstalls the release but leaves the PostgreSQL PVC and immutable Secrets. Redeploying reuses the database and validates the existing migration history.

Permanent deletion is intentionally explicit:

```sh
dev/ctl purge ayni-orchestrator-dev
```

That command refuses unlabelled namespaces and deletes the namespace, including retained PostgreSQL data.
