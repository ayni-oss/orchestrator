# Live development

This is a development-only loop for Docker Desktop Kubernetes. It runs the sole
example-derived web application with Svelte HMR and rebuilds/redeploys the Rust
backend. There is no separate diagnostic frontend. Immutable-image acceptance is
still required; live debug builds are development evidence only.

## Application database baseline

The fresh-install schema is organized into 14 layer migrations, `001`–`014`.
Each table's final columns, constraints and indexes live in its owning layer;
repeated alterations from the former 41-step history are folded into that definition.
The former history and the temporary ordinal-42 baseline are incompatible. They
are rejected before schema or data changes; `live migrate` cannot upgrade them.
Keep existing installations and their retained data intact and use a new isolated
local namespace. Deleting old volumes remains a separate, explicit operator action.
After installation, migrations are immutable; new changes start at `015`.

## Start

Start Docker Desktop and enable Kubernetes in its settings. Wait for Kubernetes
to be ready. Install Make, Python 3 and the pinned prerequisites in the
[local development guide](../docs/technical/operations/local-development.md),
then run these commands from the repository root:

```sh
kubectl config current-context
kubectl --context docker-desktop get --raw=/readyz
make dev
```

The first command should print `docker-desktop` and the second should print
`ok`. If another context is active, select `docker-desktop` yourself before
starting. The live loop checks the context and leaves your selection alone.
`make dev` installs the locked frontend dependencies and runs `dev/ctl live`
in the foreground.

The default namespace is **ayni-orchestrator-live**, separate from ordinary dev and
acceptance deployments. Set `AYNI_ORCHESTRATOR_NAMESPACE` consistently for every
command when selecting another disposable local namespace. Only the explicitly
selected `docker-desktop` context is allowed. Namespace names must start with
`ayni-orchestrator-` and fit the 54-character parent/session bound.

Run `make dev-status` from a second terminal. Wait for `"phase": "ready"` before
opening http://127.0.0.1:5174. The UI and gateway ports must be free. Configure them
with `AYNI_ORCHESTRATOR_LIVE_UI_PORT` (default 5174) and
`AYNI_ORCHESTRATOR_LIVE_GATEWAY_PORT` (default 3000). Edit the existing application
in `apps/web` to exercise HMR. Inspect backend build/rollout state with
`dev/ctl live status` and the terminal; no extra UI or server is required. The
application forwards only allowlisted requests to the loopback API gateway.

## Smoke-test the web app and backend

1. On the home page, create a workspace with a test name.
2. Open that workspace and create a project.
3. Reload the page and reopen the workspace. Confirm that the project is still
   listed. This exercises writes and reads through the web app, local gateway,
   Kubernetes control plane and PostgreSQL. The test records remain in the
   selected namespace.

To check the backend independently, run this from the second terminal:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-live dev/ctl check
```

Use your selected live namespace if you overrode the default. If the UI reports
an API error, check the live terminal and `make dev-status` for build or rollout
errors.

## Rebuilds and shutdown

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
pnpm --filter @ayni-orchestrator/web check
pnpm --filter @ayni-orchestrator/web test
pnpm --filter @ayni-orchestrator/web test:hmr
```

Before claiming a delivery validated, run normal repository checks and build/deploy
immutable production images from the recorded source revision using `dev/ctl deploy`
in a separate acceptance namespace. Run the relevant cumulative acceptance profiles.
Debug images, successful build status and an HMR update are development evidence only.

## Product UI

`apps/web` is the only frontend. It uses the example's shared shell, original
light/dark theme tokens, fonts and component library. The live loop and production
build serve the same application; no diagnostic page or alternate design remains.
The reconnecting headless gateway lives in `dev/lib/gateway.py`.
