# Local durable workflow stack

The supported Docker Desktop stack contains one Ayni control plane, PostgreSQL 17.6, an isolated authenticated three-stream JetStream server, and one retained scripted executor. PostgreSQL is authoritative; broker storage and consumer ACKs remain distinct from application acceptance and executor ownership.

## Deploy and exercise

```sh
dev/ctl deploy
dev/ctl check --expect-empty
dev/ctl scenario
```

The first deploy creates distinct immutable database, application, executor, acceptance, and administrative Secrets without printing their values. Credentials appear only in Secret-backed files or `secretKeyRef` entries. PostgreSQL, JetStream, and the executor ledger each use retained storage.

`dev/ctl scenario` runs the private-v1 diagnostic inside the control-plane Pod and verifies one admitted message reaches one accepted scripted response, reconnecting its watch after every update. See [local development](../docs/technical/operations/local-development.md) for the stream limits, ACL boundary, manual port-forwarding, restart checks, and complete validation commands.

## Retention and teardown

`dev/ctl cleanup` uninstalls the release but preserves authoritative PVCs and immutable Secrets. Redeployment validates and reuses them. Do not roll back across an unresolved handoff; repair forward or intentionally reset the disposable namespace.

Permanent deletion is explicit:

```sh
dev/ctl purge ayni-orchestrator-dev
```

The command refuses absent or unlabelled namespaces and deletes all retained local workflow data.
