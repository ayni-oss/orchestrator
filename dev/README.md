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

The env file is optional. With no file or incomplete credential groups, deployment
skips those groups and leaves their setup to the UI. Previously provisioned Secrets
are preserved. Complete groups remain administrator-managed; the bootstrap refuses
to overwrite UI-managed Secrets.

## Hot reload

Install [Tilt](https://docs.tilt.dev/install.html) once, then start the
development-only live-update environment with:

```sh
dev/ctl hot-reload
```

This first synchronizes Secrets and deploys the normal Helm release with local
development images. It then attaches Tilt to the existing `control-plane` Pod;
Helm remains the resource owner. Each new Tilt session first synchronizes the current
control-plane and UI source, including edits made before the session started.
The control plane is compiled before Tilt attaches, so generated Rust API bindings
use the current protobuf definitions. The Pod continues to contain both sidecars.
Only these development images have a writable root filesystem, allowing Tilt to
synchronize source beside their development dependencies; normal images remain
read-only. The web development container has a 1 GiB memory limit because Vite
optimizes the full Svelte dependency graph at startup; this higher allocation
is confined to the hot-reload overlay.

- Changes under `backend/` or `proto/` are copied into the control-plane
  container and compiled there. A development-only supervisor restarts the
  control-plane process without restarting its container, retaining synchronized
  source and the newly compiled binary. Container termination forwards the signal
  to the child process and exits normally.
  Initial synchronization and subsequent live updates compile with one Cargo job
  to keep compiler concurrency within the control-plane container's 3 GiB memory
  budget while the server remains running.
  Tilt allows five minutes for initial attachment, which includes that build;
  its default 30-second Kubernetes apply timeout is too short for Rust rebuilds.
- Changes under `apps/web/`, `packages/bff/`, or `clients/typescript/` are
  copied into the web container; Vite refreshes the browser without a Pod
  rollout. The web application is available at http://127.0.0.1:18080.
- JavaScript package manifests, chart values, Secrets,
  or Helm templates are deployment inputs. Stop Tilt and run `dev/ctl
  hot-reload` again after changing them.

Use `dev/ctl cleanup` to uninstall the Helm release when finished. `tilt down`
only stops live updates and deliberately leaves Helm-owned resources intact.

`deploy` and its `redeploy` alias safely re-run in place: they reconcile the
selected private env file into Kubernetes Secrets, rebuild all three project
images, load them into Docker Desktop Kubernetes, and run `helm upgrade
--install`. Use `secrets` when only credential values need to be refreshed.
The **Certificates** sidebar page is the preferred setup for dedicated Kubernetes
environment builds. Generate an Ed25519 signing key or import a 64-character
lowercase hexadecimal seed with its key ID. Kubernetes stores private material;
the UI displays only public trust policy. The first certificate becomes the global
default. Ayni runtime settings can follow that default or select another certificate;
projects inherit their repositories’ runtime selections. Custom runtimes do not
use certificates. Until a usable certificate exists, signed builds require setup.

For administrator-managed certificates, the mode-600 `dev/.env` may still contain
`AYNI_ENV_CERTIFICATE_SIGNING_KEY` and `AYNI_ENV_CERTIFICATE_KEY_ID`.
`dev/ctl secrets` provisions the `environment-producer` Secret. The next `deploy`
or `hot-reload` registers that explicit local Secret through the chart’s
`connections.adminCertificates` configuration using a name-only existence check.
When it is absent, deployment leaves certificate setup to the UI. The UI presents
read-only administrator details and detects replacements. Legacy `build.signing_secret` references
are deprecated and registered for compatibility. Signing seeds reach only trusted
builder Jobs. Trust-update jobs and coding workers receive no signing authority.
Review and merge the draft public trust-policy/lock update proposed for affected
Ayni repositories before the new certificate’s environment becomes ready.

For private GHCR runtime images, optionally set `GHCR_PULL_USERNAME` and
`GHCR_PULL_TOKEN` (a classic personal access token with `read:packages`) in the
same mode-600 `dev/.env`. `dev/ctl secrets` creates the Kubernetes
`runtime-pull` imagePullSecret. Reference `runtime-pull` as the runtime
`pull_secret`; it is used by Kubernetes to fetch a runtime image before the
container starts and is never mounted into the application. Add a distinct
`GHCR_PUSH_TOKEN` to create the separate `runtime-push` Docker config Secret.
Deployment registers the GHCR connection only when both admin Secrets exist and
valid namespace metadata is present. `GHCR_IMAGE_NAMESPACE` selects the owner
namespace for connection probes and is required when `GHCR_PUSH_TOKEN` is
supplied. Runtime images use `ghcr.io/<repository-owner>/<repository>`; the
credential username is used only for authentication. Registration removes the `ghcr.io/` host prefix before
sending the namespace to the control plane. Push credentials reach only trusted build/probe Jobs.
Complete GitHub App ID/key credentials register automatically on the next deploy;
installation bindings are still chosen through repository setup. OpenAI is registered
only when the bootstrap Secret exists; otherwise configure it in LLM providers.

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
| `dev/ctl test jetstream` | Validate local broker persistence, bounded saturation and metrics in one disposable namespace with automatic cleanup |

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
  NAME`. Managed runtime proof comes from the actual environment build; task quality
  is checked by the worker on the managed topology.

Long-form behavior, security boundaries, and troubleshooting remain in
[`docs/technical/operations/local-development.md`](../docs/technical/operations/local-development.md) and
[`docs/technical/operations/operations.md`](../docs/technical/operations/operations.md).

The local stack includes persistent NATS JetStream and its Grafana dashboard.
Normal cleanup retains `jetstream-data`; `jetstream.enabled=false` disables the
transport independently. See [persistent local JetStream](../docs/technical/operations/local-development.md#persistent-local-jetstream)
for limits, replay expiry, trusted-development access and recovery boundaries.

### Connection onboarding and rotation

Start without GitHub/GHCR credentials and onboard them in Source control and
Container registries. GitHub installation permissions remain managed in GitHub.
Private registry images require a separate pull-only token; push credentials are
reserved for trusted builders and probes. UI credential inputs clear after submission.
The control plane reconciles Secret changes every 30 seconds; UI writes trigger
immediate reconciliation and validated activation without a pod restart.

`dev/ctl secrets` optionally creates administrator GitHub and GHCR Secrets and
labels them as admin-managed; it refuses to overwrite a UI-managed Secret. Deployment
registers complete bootstrap connections from safe Secret metadata, without passing
credentials into Helm. Local
managed runtime defaults to native `linux/arm64` and disables architecture emulation;
Intel hosts must select `linux/amd64` explicitly. A connection test requires
`connections.testImage` set to a digest-pinned control-plane image.

Database schema changes reject old state rather than migrate it. Inventory the
exact retained claims and obtain explicit approval before deleting application
data; runtime outputs and administrator Secrets are separate retained resources.
Use an isolated Docker Desktop namespace to validate fresh onboarding safely.


To exercise the accepted-event publisher, load the current production control-plane
image and an existing web image, then run `dev/tests/outbox.sh CONTROL_PLANE_IMAGE WEB_IMAGE`.
The fixed disposable namespace and its PVCs are removed on success or failure.
Test-only image tags should also be removed from the host and Docker Desktop node.

For the transport recovery gate, run `dev/ctl test recovery CONTROL_PLANE_IMAGE WEB_IMAGE`
with both current production images loaded into Docker Desktop. It shares the
publisher fixture and uses a separate `ayni-orchestrator-recovery-test` namespace.
A bounded test-only NATS proxy uses the web image's Node runtime to suppress actual
publisher storage replies and the accepted fact's consumer ACK before it reaches
NATS. Proxy counters must prove both faults occurred. Production configuration
has no fault switches.

The gate verifies accepted SQLite state through stream saturation, lost publisher
ACK retry, committed consumer receipt with lost ACK, retained durable cursor across
broker/application restart, and identical-byte replay after broker deduplication
expires. Filling the real receipt table must leave at most eight broker deliveries
in flight; releasing only synthetic fixture diagnostics must drain all deliveries.
Every stopped-state inspection compares retained acceptance, projections, action
counts, execution identity and failure budgets. No broker consumer is deleted to
force this gate's replay. The fixture disables runtime/provider execution; it
validates delivery cannot create actions or reset existing cleanup/backoff, rather
than claiming provider-side exactly-once effects. Provider recovery remains a
separate per-flow cutover gate.

The proxy parser has standalone segmentation, selective-drop and buffer-budget
tests (`node --test dev/tests/nats-fault-proxy.test.mjs`), also run by PR validation.
Namespace/PVC cleanup runs on failure as well as success. Remove the gate's own
host and node image tags after use; avoid broad Docker pruning.
