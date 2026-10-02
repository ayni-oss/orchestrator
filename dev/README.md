# Local development

Everything specific to the supported Docker Desktop development environment lives
in this directory. The reusable application chart remains under
`deploy/charts/ayni-orchestrator`; it has no dependency on this directory. The
dependency points one way: `dev/charts/ayni-orchestrator-local` composes the reusable chart with local
monitoring and validation fixtures.

## Quick start

Prerequisites are Docker Desktop Kubernetes, the `docker-desktop` context, Helm
4.1.1, `kubectl`, Docker, Git, `curl`, and `grpcurl`.

```sh
dev/ctl init
# Fill dev/.env without printing or committing it.
dev/ctl secrets
dev/ctl deploy
dev/ctl check
```

`deploy` and its `redeploy` alias safely re-run in place: they rebuild all three
project images, load them into Docker Desktop Kubernetes, and run
`helm upgrade --install`. Credential updates stay explicit through `secrets`.

Use a different ignored env file when working with an isolated profile:

```sh
dev/ctl init --env-file dev/.env.alice
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-alice \
  dev/ctl secrets --env-file dev/.env.alice
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-alice dev/ctl deploy
```

## Lifecycle

| Command | Effect |
| --- | --- |
| `dev/ctl secrets` | Parse the private env file and update local Kubernetes Secrets |
| `dev/ctl deploy` | Build and install or redeploy the complete stack in place |
| `dev/ctl check` | Check application, gRPC, browser, metrics, and dashboards |
| `dev/ctl status` | Show the release, Pods, and retained claims |
| `dev/ctl cleanup` | Uninstall the release while preserving Secrets, Jobs, and retained claims |
| `dev/ctl purge ayni-orchestrator-dev` | Permanently remove the labeled namespace and all local state |
| `dev/ctl render CP WORKER WEB` | Render the complete Helm composition for inspection |
| `dev/ctl dependencies` | Rebuild ignored chart archives from `Chart.lock` |

`purge` is deliberately explicit because it deletes credentials and retained
product data. None of these commands changes the current Kubernetes context,
resets Docker Desktop, deletes shared Docker volumes, or performs a broad Docker
prune.

The deployment path requires a clean committed checkout so built image labels
and tags have unambiguous source provenance.

## Layout

- `.env.example` documents the only accepted local credential keys.
- `.env` and `.env.*` are ignored and must never be committed. If upgrading
  from the former root layout, manually move `.env` to `dev/.env` and run
  `chmod 600 dev/.env`; tooling never inspects or copies the old credential file.
- `charts/ayni-orchestrator-local/` is the Docker Desktop umbrella chart and
  local-only configuration.
- `scripts/` contains the three command implementations behind `dev/ctl`.
- `lib/` contains shared shell functions; neither directory is a public command
  surface.
- `tests/` contains opt-in local validation drivers, exposed as `dev/ctl test
  NAME`. Managed runtime and assessment proof is owned by the reviewed Actions
  workflows, not a local Docker-in-Docker fixture.

Long-form behavior, security boundaries, and troubleshooting remain in
[`docs/local-development.md`](../docs/local-development.md) and
[`docs/operations.md`](../docs/operations.md).
