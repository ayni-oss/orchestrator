# Local development

The supported M1 Slice 3 stack runs one control plane, PostgreSQL 17.6, an isolated authenticated NATS/JetStream server, and one retained scripted executor on Docker Desktop Kubernetes. PostgreSQL is the workflow authority; JetStream is durable transport, and broker storage never implies executor ownership.

## Prerequisites

- Docker Desktop Kubernetes with current context `docker-desktop`;
- Helm 4.1.1;
- `kubectl`, Docker, OpenSSL, Rust 1.94, Node 24.14, pnpm 10.31, and Buf 1.71;
- at least 8 GiB free in the Docker Desktop VM before building the two Rust workload images.

The capacity guard does not prune caches, volumes, or unrelated data. Reclaiming Docker storage is an explicit operator decision.

## Deploy and check

```sh
dev/ctl deploy
dev/ctl check --expect-empty
dev/ctl scenario
```

The first deploy creates the namespace and seven immutable Secrets without printing their values:

- `workflow-postgres-admin`, mounted only into PostgreSQL;
- `workflow-postgres-migrator`, mounted only into the migration init container;
- `workflow-postgres-runtime`, mounted only into the serving container;
- `workflow-jetstream-admin`, mounted only into NATS and the stream-provisioning hook;
- `workflow-jetstream-application`, mounted into NATS and the control plane;
- `workflow-jetstream-executor`, mounted into NATS and the executor; and
- `workflow-jetstream-acceptance`, mounted only into NATS for the later acceptance harness.

The executor Secret also owns its fixed local instance ID and Ed25519 seed. The private seed never leaves that Pod or enters broker events. Anonymous access is disabled. Workload ACLs allow only the contracted command, report, targeted-decision, consumer-info/fetch, and ACK subjects; stream administration remains exclusive to the administrative identity.

The pinned broker provisions three file-backed, `discard=new` streams:

| Stream | Subject | Limit |
| --- | --- | --- |
| `ORCHESTRATOR_WORKFLOW_COMMAND_V1` | `orchestrator.workflow.command.conversation.v1` | 25,000 / 256 MiB |
| `ORCHESTRATOR_WORKFLOW_REPORT_V1` | `orchestrator.workflow.report.conversation.v1` | 50,000 / 192 MiB |
| `ORCHESTRATOR_WORKFLOW_DECISION_V1` | `orchestrator.workflow.accepted.conversation.v1.*` | 50,000 / 64 MiB |

All use seven-day age limits, a two-minute duplicate window, one local replica, and a 260-KiB maximum stored message. The combined 512-MiB stream budget sits below the 1-GiB server budget on a retained 2-GiB PVC. The executor has a retained 512-MiB PVC and enforces a 192-MiB authority ledger plus a 64-MiB quarantine subquota within a 256-MiB combined logical cap.

`dev/ctl check` verifies workload readiness, migration history through version 7, the exact stream limits, retained storage, and executor-ledger initialization. `--expect-empty` additionally refuses a database that already contains a workspace.

## Contributor console

After deploying and checking the stack, run:

```sh
dev/ctl console
```

Open `http://127.0.0.1:3000`. Create or select a workspace, project and conversation,
then send a message. The console shows accepted messages, scripted responses,
activity, waiting states, stable older history, and receipt/cursor diagnostics.
Use Disconnect/Reconnect to exercise observation without cancelling admitted work.
Refresh reloads the selected conversation from authoritative state. Open another
tab to inspect the same conversation or select a different one.

A failed command remains in that tab's pending panel. Retry sends its original
key and exact content; do not clear browser storage to retry an ambiguous command.
The gateway rejects foreign origins and hostnames, so use the printed numeric
loopback URL. Stop with Ctrl-C. After a control-plane Pod restart, rerun the console
command to forward the replacement Pod. `dev/ctl console 3001` selects another port.

To repeat browser acceptance against the running console:

```sh
pnpm exec playwright install chromium
pnpm console:acceptance
```

The scenario creates a separate workspace and retains its accepted data. It checks
scripted responses, refresh, two tabs, reconnect, a lost command response followed
by reload/retry, conversation isolation, older history, content escaping and mobile
layout. Browser-boundary fixtures additionally check typed cursor expiry and a
bounded slow-reader replay burst; real retention/backpressure is covered by the
cumulative PostgreSQL gate. `CONSOLE_URL=http://127.0.0.1:3001` selects another local console port;
`CONSOLE_EVIDENCE_DIRECTORY=/absolute/path` additionally saves a screenshot.

The console is a trusted local contributor surface using the scripted executor;
provider configuration and real agent sessions arrive in M2. See [console architecture](../architecture/contributor-console.md)
for the transport decision, bounds, security and recovery semantics.

## Happy-path diagnostic

`dev/ctl scenario` executes the credential-free diagnostic inside the control-plane Pod. It uses the pod-loopback private-v1 listener to create a workspace, project, and conversation; admit one message; observe the signed executor claim and accepted scripted response through a watch, reconnect after each update; and verify the terminal snapshot and stable history. It prints generated resource IDs and the final revision, never credentials or message content.

For manual access, port-forward the Pod rather than the Service:

```sh
pod=$(kubectl --context docker-desktop -n ayni-orchestrator-dev get pod \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
  -o jsonpath='{.items[0].metadata.name}')
kubectl --context docker-desktop -n ayni-orchestrator-dev \
  port-forward "pod/$pod" 50051:50051

cargo run -p ayni-orchestrator-control-plane --bin workflow-diagnostic
```

Health and readiness remain separate:

```sh
kubectl --context docker-desktop -n ayni-orchestrator-dev \
  port-forward service/control-plane 8080:8080
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
curl --fail http://127.0.0.1:8080/metrics
```

Broker unavailability leaves PostgreSQL readiness honest and delivery pending. PostgreSQL unavailability makes the command/snapshot boundary unavailable.

## Watch recovery and retention

Keep the last received cursor to resume `WatchConversation`. An empty cursor starts with a consistent snapshot; a valid cursor starts with the next durable update. Heartbeats have no cursor. A disconnected watch never cancels admitted work.

`WATCH_CURSOR_EXPIRED` means the cursor predates the seven-day watermark. Reconnect without a cursor to recover authoritative current state. `RESOURCE_EXHAUSTED` closes a slow reader; resume from its last consumed cursor. Cleanup bounds each pass and preserves immutable receipts and unresolved transport evidence.

`/readyz` names a full application report quarantine or unsupported retained version. Stop the incompatible producer, deploy a compatible consumer, and allow retained replay to resolve the entries. Malformed entries retain identity/digest diagnostics for eight days; unsupported exact bytes have no automatic expiry.

## Restart and retention check

After a successful scenario, restart each owner and rerun the checks:

```sh
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart statefulset/workflow-postgres
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart statefulset/workflow-jetstream
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart deployment/control-plane
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart statefulset/scripted-executor

dev/ctl check
dev/ctl scenario
```

PostgreSQL, JetStream, and executor-ledger PVCs survive ordinary restarts and Helm uninstall. The control plane reruns schema/checksum/privilege validation, exact outbox bytes remain authoritative until claim acceptance, and the executor resumes retained reports before accepting new work.

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

The PostgreSQL gate covers migration version 7, signed claim/terminal acceptance, duplicate report decisions, accepted response insertion, and conversation-only FIFO context promotion in addition to the Slice 1 authority checks. The Helm gate validates pinned broker images, exact stream budgets, role ACL shape, Secret separation, and retained PVCs. `dev/ctl scenario` is the deployable Docker Desktop evidence.

## Cleanup

```sh
dev/ctl cleanup
```

Cleanup removes the Helm release while preserving PostgreSQL, JetStream, executor-ledger PVCs, and immutable Secrets. A later deploy reuses them. Do not roll back while a command/report/decision handoff is unresolved; repair forward or intentionally reset the disposable namespace.

Permanent deletion requires the exact local namespace:

```sh
dev/ctl purge ayni-orchestrator-dev
```

The purge command refuses absent, unlabelled, or non-local namespaces. It deletes the namespace, all Secrets, and all three retained stores.
