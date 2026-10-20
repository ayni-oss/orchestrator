# Local development

The supported local stack is one control plane and one PostgreSQL 17.6 StatefulSet on Docker Desktop Kubernetes. PostgreSQL is the sole workflow authority. The local chart does not deploy browser, worker, broker, provider, or custom-client components.

## Prerequisites

- Docker Desktop Kubernetes with current context `docker-desktop`;
- Helm 4.1.1;
- `kubectl`, Docker, OpenSSL, Rust 1.94, Node 24.14, pnpm 10.31, and Buf 1.71;
- at least 8 GiB free in the Docker Desktop VM before building the control-plane image.

The capacity guard does not prune caches, images, volumes, or unrelated data. Reclaiming Docker storage is an explicit operator decision.

## Deploy and check

```sh
dev/ctl deploy
dev/ctl check --expect-empty
```

The first deploy creates the namespace and three immutable Secrets without printing their values:

- `workflow-postgres-admin`, mounted only into PostgreSQL;
- `workflow-postgres-migrator`, whose `url` file is mounted only into the startup migration init container;
- `workflow-postgres-runtime`, whose `url` file is mounted only into the serving container.

The PostgreSQL image is pinned to the reviewed 17.6 manifest digest. An init container gives UID 999 ownership of a new retained volume and skips recursive traversal once UID 999 already owns it, so restarts preserve the hardened `0700` data directory. First initialization creates distinct migration and runtime roles. The migration runner then grants explicit application-table DML and sequence use while keeping migration history read-only to the runtime role; blanket default grants are not used.

`dev/ctl check` waits for both workloads, checks migration version 1, and optionally verifies that no workspace has been admitted. It never reads or prints credential values.

## Trusted diagnostic

Workflow gRPC binds to pod loopback and is not part of the Kubernetes Service. In one terminal:

```sh
pod=$(kubectl -n ayni-orchestrator-dev get pod \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
  -o jsonpath='{.items[0].metadata.name}')
kubectl -n ayni-orchestrator-dev port-forward "pod/$pod" 50051:50051
```

In another terminal, create the hierarchy, admit one message, and verify its snapshot and history:

```sh
cargo run -p ayni-orchestrator-control-plane --bin workflow-diagnostic
```

The diagnostic accepts only an HTTP loopback socket through `--endpoint`. It prints generated resource IDs and the final revision, never credentials or message content.

Health and readiness are separate:

```sh
kubectl -n ayni-orchestrator-dev port-forward service/control-plane 8080:8080
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

## Restart and retention check

After running the diagnostic, record its generated conversation ID, restart both workloads, and query that conversation again with the snapshot RPC:

```sh
kubectl -n ayni-orchestrator-dev rollout restart statefulset/workflow-postgres
kubectl -n ayni-orchestrator-dev rollout status statefulset/workflow-postgres
kubectl -n ayni-orchestrator-dev rollout restart deployment/control-plane
kubectl -n ayni-orchestrator-dev rollout status deployment/control-plane
dev/ctl check

# With the pod port-forward running again, read the exact retained ID.
grpcurl -plaintext \
  -import-path proto \
  -proto ayni/orchestrator/v1/project_conversations.proto \
  -d "{\"workspace_id\":\"$workspace_id\",\"project_id\":\"$project_id\",\"conversation_id\":\"$conversation_id\"}" \
  127.0.0.1:50051 \
  ayni.orchestrator.v1.ProjectConversationService/GetConversationSnapshot
```

The returned revision and admitted message must match the pre-restart diagnostic result. A PostgreSQL restart keeps the same PVC. The control plane reruns the advisory-lock migration check and refuses readiness if history, checksum, privileges, or any catalog-manifest object is invalid.

## Validation gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
scripts/ci/check-workflow-postgres-feasibility.sh
scripts/ci/check-workflow-private-v1-feasibility.sh
scripts/ci/check-helm.sh
pnpm proto:check
pnpm docs:check
```

The PostgreSQL gate starts a disposable digest-pinned database and covers migrations, concurrent locking, rollback, scoped idempotency, FIFO admission, capacity, exact outbox bytes, snapshots/history, readiness, and gRPC adapters.

## Cleanup

```sh
dev/ctl cleanup
```

Cleanup removes the Helm release while preserving the StatefulSet-created PVC and immutable Secrets. A later deploy reuses the database.

Permanent deletion requires the exact local namespace:

```sh
dev/ctl purge ayni-orchestrator-dev
```

The purge command refuses absent, unlabelled, or non-local namespaces. It deletes the namespace, Secrets, and retained PostgreSQL data.
