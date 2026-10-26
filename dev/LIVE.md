# Live development

This is a development-only loop for Docker Desktop Kubernetes. It does not qualify
production images or advertise a supported product UI. The diagnostic page is a
small HMR and canonical API probe; repository onboarding will supply the product UI.

## Start

Install the pinned prerequisites in the [local development guide](../docs/technical/operations/local-development.md),
then install workspace dependencies and start the loop:

```sh
pnpm install --frozen-lockfile
dev/ctl live
```

The default namespace is **ayni-orchestrator-live**, separate from ordinary dev and
acceptance deployments. Set `AYNI_ORCHESTRATOR_NAMESPACE` consistently for every
command when selecting another disposable local namespace. Only the explicitly
selected `docker-desktop` context is allowed. Namespace names must start with
`ayni-orchestrator-` and fit the 54-character parent/session bound.

Open http://127.0.0.1:5173. Both ports must be free; select alternatives with
`AYNI_ORCHESTRATOR_LIVE_UI_PORT` (default 5173) and
`AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT` (default 3000). The page shows build state, last successful deployment
identity, stopped/stale state and a read-only workspace probe. Edit
`dev/live-ui/main.mjs` to exercise HMR. The local Vite proxy validates Host/Origin
before forwarding requests to the loopback-only API gateway; private gRPC remains
pod-local. No credentials are returned through the page or status file.

Rust edits are detected by content, including additions and deletions. The loop
builds affected component images with pinned Linux Rust and BuildKit Cargo caches,
imports content-addressed local tags into Docker Desktop, then rolls out the normal
workloads. The control-plane gateway is also rebuilt for host execution. A failed
compile leaves deployed services running. A failed rollout can partially update
services; the status retains the last fully successful deployment and reports the
failure. Inspect actual Pods before assuming all services still run that build.
Edits made during compilation prevent deployment of that build and trigger another.
The first cold compilation can take several minutes; subsequent builds reuse caches.
Images are local only and are not automatically pruned.

Database, broker and supervisor storage survive reloads and Ctrl-C. Running workers
retain their original executable/image and generation; newly provisioned workers use
the new image. Editing code does not cancel tasks, clear uncertainty, replace a
worker's frozen inputs or authorize external operations. The gateway reconnects to a
ready Pod after rollout; browser watches must reconcile through canonical reads.
Provider/fixture settings in the existing Helm release are preserved.

Ctrl-C stops the watcher, UI and local gateway/port-forward. It leaves the cluster
running. To stop the deployment while retaining its stores:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-live dev/ctl cleanup
```

## Status and one-shot builds

```sh
dev/ctl live status
dev/ctl live once
```

`once` builds and deploys without starting the UI or gateway. Build logs go to the
terminal. Ignored `.local/live/NAMESPACE/status.json` retains desired/active source
hashes, image tags, HEAD revision, timestamps and the latest phase. The source hashes
cover uncommitted changes; HEAD alone is not the build identity. One command owns a
namespace at a time. This is a local checkout lock, not a multi-machine controller.
Changes to charts, deployment scripts and development tooling require stopping and
restarting the loop; the automatic reload scope is Rust inputs and frontend files.

## Migrations and branch switches

Migration SQL and the embedded migration/privilege validator are a reload barrier.
After the first deployment, changing either blocks automatic backend rollout.
Stop the live loop and explicitly run:

```sh
dev/ctl live migrate
dev/ctl live
```

`migrate` builds current code, scales the control plane down, then deploys it with
the normal migration init container before restoring the local singleton replica. Runtime
and migrator credentials remain separate. Workers may temporarily lose the authority
endpoint; their retained identity and reports must reconcile after restart. Avoid
schema experiments during externally effectful work; use a separate disposable
namespace when needed. Migration failure is visible and may leave serving unavailable;
there is no automatic schema rollback or data deletion.

Applied migrations remain immutable. Editing an applied SQL file fails its existing
checksum validation. Add a forward migration, use another namespace, or deliberately
reset a disposable environment. A fresh checkout without a status baseline performs
the normal initial deployment/migration checks; failure still cannot reset data.

```sh
# Stop live first. Permanently deletes the exact owned environment and sessions.
dev/ctl live reset ayni-orchestrator-live
dev/ctl live
```

Reset delegates to the existing ownership-checked purge and deletes coupled database,
broker, supervisor and session stores together. It never deletes just PostgreSQL or
silently repairs checksums. Recreate test data through canonical APIs, for example:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-live dev/ctl scenario
```

Switching to an older/incompatible schema requires a compatible namespace or explicit
reset. Do not use an environment containing work you need to preserve for that loop.

## Validation

```sh
python3 -m unittest discover -s dev/tests -v
python3 -m unittest discover -s dev/live-ui -p 'test_*.py' -v
pnpm --filter @ayni-orchestrator/live-ui test
```

Before claiming a delivery validated, run normal repository checks and build/deploy
immutable production images from the recorded source revision using `dev/ctl deploy`
in a separate acceptance namespace. Run the relevant cumulative acceptance profiles.
Debug images, a green status page and an HMR update are development evidence only.
