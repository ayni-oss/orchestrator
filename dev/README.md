# Local durable workflow stack

> UI status: the console and initial product workspace are removed. Browser steps
> and evidence below describe the retired UI and are historical reference only.
> Browser-dependent acceptance profiles are unavailable until replacement coverage
> is implemented. The current local API adapter is `dev/ctl gateway`; it serves
> no UI. See [the retained UI example](../examples/web/EXAMPLE.md).

The supported Docker Desktop stack contains one Ayni control plane, PostgreSQL 17.6, an isolated authenticated three-stream JetStream server, and one retained scripted executor. PostgreSQL is authoritative; broker storage and consumer ACKs remain distinct from application acceptance and executor ownership.

## Deploy and exercise

```sh
dev/ctl deploy
dev/ctl check --expect-empty
dev/ctl scenario
dev/ctl gateway
```

The first deploy creates distinct immutable database, application, executor, acceptance, and administrative Secrets without printing their values. Credentials appear only in Secret-backed files or `secretKeyRef` entries. PostgreSQL, JetStream, and the executor ledger each use retained storage.

`dev/ctl scenario` runs the private-v1 diagnostic inside the control-plane Pod and verifies one admitted message reaches one accepted scripted response, reconnecting its watch after every update. See [local development](../docs/technical/operations/local-development.md) for the stream limits, ACL boundary, manual port-forwarding, restart checks, and complete validation commands.

## Cumulative operational acceptance

Use an owned `ayni-orchestrator-acceptance-*` namespace with scripted task execution,
the task-quality fixture runtime, installed Playwright Chromium and committed source.
Build and load a digest-pinned managed runtime for the full Orchestrator checkout, then run:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-example \
AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE=docker.io/local/orchestrator-runtime@sha256:REPLACE_WITH_ACTUAL_DIGEST \
  dev/ctl acceptance /absolute/new/evidence-directory --m8
```

The profile requires cumulative M4/M6, dependency integration and repair, worker recovery,
assessment, both MCP/provider fixtures, diagnostics and own-source managed self-development.
Reports preserve live-provider and production-capacity gaps separately.
See [operational acceptance](../docs/technical/operations/operational-acceptance.md) for gates and evidence interpretation.

Individual deployed profiles accept a new absolute evidence directory:

```sh
dev/scripts/dependency-acceptance.sh /absolute/new/dependency-evidence
dev/scripts/worker-recovery-acceptance.sh /absolute/new/recovery-evidence
dev/scripts/assessment-acceptance.sh /absolute/new/assessment-evidence
dev/fixtures/mcp/acceptance.sh LOCAL_EXECUTOR_IMAGE anthropic /absolute/new/mcp-evidence
```

## Retention and teardown

`dev/ctl cleanup` uninstalls the release but preserves authoritative PVCs and immutable Secrets. Redeployment validates and reuses them. Do not roll back across an unresolved handoff; repair forward or intentionally reset the disposable namespace.

Permanent deletion is explicit:

```sh
dev/ctl purge ayni-orchestrator-dev
```

The command refuses absent or unlabelled namespaces and deletes all retained local workflow data.

`dev/ctl gateway` serves the local JSON/SSE API adapter at `http://127.0.0.1:3000` through a loopback-only gateway and Pod port-forward. It serves no browser UI. Stop with Ctrl-C; rerun after a Pod restart.
