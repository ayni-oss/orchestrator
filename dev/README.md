# Local development

Everything specific to the supported Docker Desktop development environment lives
in this directory. The reusable application chart remains under
`deploy/charts/ayni-orchestrator`; it has no dependency on this directory. The
dependency points one way: `dev/charts/ayni-orchestrator-local` composes the reusable chart with local
monitoring.

## Quick start

Prerequisites are Docker Desktop Kubernetes, the `docker-desktop` context, Helm
4.1.1, `kubectl`, Docker, Git, Python 3, `curl`, and `grpcurl`.

```sh
dev/ctl init
# Fill dev/.env without printing or committing it.
dev/ctl secrets
dev/ctl deploy
dev/ctl check
```

## Hot reload

Install [Tilt](https://docs.tilt.dev/install.html) once, then start the
development-only live-update environment with:

```sh
dev/ctl hot-reload
```

This first synchronizes Secrets and deploys the normal Helm release with local
development images. It then attaches Tilt to the existing `control-plane` Pod;
Helm remains the resource owner. The Pod continues to contain both sidecars.
Only these development images have a writable root filesystem, allowing Tilt to
synchronize source beside their development dependencies; normal images remain
read-only. The web development container has a 1 GiB memory limit because Vite
optimizes the full Svelte dependency graph at startup; this higher allocation
is confined to the hot-reload overlay.

- Changes under `backend/` or `proto/` are copied into the control-plane
  container, compiled there, and restart only that container.
- Changes under `apps/web/`, `packages/bff/`, or `clients/typescript/` are
  copied into the web container; Vite refreshes the browser without a Pod
  rollout. The web application is available at http://127.0.0.1:18080.
- `Cargo.lock`, package manifests, generated bindings, chart values, Secrets,
  or Helm templates are deployment inputs. Stop Tilt and run `dev/ctl
  hot-reload` again after changing them.

Use `dev/ctl cleanup` to uninstall the Helm release when finished. `tilt down`
only stops live updates and deliberately leaves Helm-owned resources intact.

`deploy` and its `redeploy` alias safely re-run in place: they reconcile the
selected private env file into Kubernetes Secrets, rebuild all three project
images, load them into Docker Desktop Kubernetes, and run `helm upgrade
--install`. Use `secrets` when only credential values need to be refreshed.
Set `AYNI_ENV_CERTIFICATE_SIGNING_KEY` and `AYNI_ENV_CERTIFICATE_KEY_ID` in the
mode-600 `dev/.env` when using the dedicated Kubernetes environment builder.
`dev/ctl secrets` then provisions the `environment-producer` Secret. Reference
that name as `build.signing_secret` in the runtime profile configuration; the
build Job consumes the values without mounting them into the control-plane or
web containers. These settings are optional when builds use GitHub Actions.

For private GHCR runtime images, optionally set `GHCR_PULL_USERNAME` and
`GHCR_PULL_TOKEN` (a classic personal access token with `read:packages`) in the
same mode-600 `dev/.env`. `dev/ctl secrets` creates the Kubernetes
`runtime-pull` imagePullSecret. Reference `runtime-pull` as the runtime
`pull_secret`; it is used by Kubernetes to fetch a runtime image before the
container starts and is never mounted into the application.

Use a different ignored env file when working with an isolated profile:

```sh
dev/ctl init --env-file dev/.env.alice
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-alice \
  dev/ctl secrets --env-file dev/.env.alice
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-alice \
  dev/ctl deploy --env-file dev/.env.alice
```

## Lifecycle

| Command | Effect |
| --- | --- |
| `dev/ctl secrets` | Parse the private env file and update local Kubernetes Secrets |
| `dev/ctl deploy` | Sync Secrets, build, and install or redeploy the complete stack in place |
| `dev/ctl hot-reload` | Deploy development images once and keep server and web source synchronized with the running Pod |
| `dev/ctl check` | Check application, gRPC, browser, metrics, and dashboards |
| `dev/ctl status` | Show the release, Pods, and retained claims |
| `dev/ctl cleanup` | Uninstall the release while preserving Secrets, Jobs, and retained claims |
| `dev/ctl purge ayni-orchestrator-dev` | Permanently remove the labeled namespace and all local state |
| `dev/ctl render CP WORKER_DIGEST WEB` | Render the complete Helm composition for inspection |
| `dev/ctl dependencies` | Rebuild ignored chart archives from `Chart.lock` |

`purge` is deliberately explicit because it deletes credentials and retained
product data. None of these commands changes the current Kubernetes context,
resets Docker Desktop, deletes shared Docker volumes, or performs a broad Docker
prune.

The deployment path can build directly from a dirty local checkout. Images carry
the current commit revision and a dirty-source label; generated local tags also
include the image content ID so each rebuilt source snapshot gets its own tag.
This keeps local iteration fast while making it clear that a dirty image is not
reproducible from its commit alone.

## Layout

- `.env.example` documents the only accepted local credential keys.
- `.env` and `.env.*` are ignored and must never be committed. If upgrading
  from the former root layout, manually move `.env` to `dev/.env` and run
  `chmod 600 dev/.env`; tooling never inspects or copies the old credential file.
- `charts/ayni-orchestrator-local/` is the Docker Desktop umbrella chart and
  local-only configuration.
- `scripts/` contains command implementations behind `dev/ctl`.
- `lib/` contains shared shell functions; neither directory is a public command
  surface.
- `tests/` contains opt-in local validation drivers, exposed as `dev/ctl test
  NAME`. Managed runtime and assessment proof is owned by the reviewed Actions
  workflows, not a local Docker-in-Docker fixture.

Long-form behavior, security boundaries, and troubleshooting remain in
[`docs/local-development.md`](../docs/local-development.md) and
[`docs/operations.md`](../docs/operations.md).
