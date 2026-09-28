# Local development

The supported local profile runs the control plane and workers on Docker Desktop
Kubernetes. It includes durable task orchestration, repository preparation,
bounded coding, independent candidate validation, draft-PR publication, and the
browser interface.

## Prerequisites

- Rust 1.94.0, pinned by `rust-toolchain.toml`
- Node 24.14.0 and pnpm 10.31.0, pinned by the root package and CI configuration
- Buf 1.71.0 for protobuf formatting, linting, generation, and compatibility checks
- Docker Desktop with Kubernetes enabled
- a reachable `docker-desktop` Kubernetes context
- `kubectl`, Docker, Git, `curl`, and `grpcurl`

Local scripts always name `docker-desktop`; they never fall back to the current
context or reset the cluster. The default namespace is
`ayni-orchestrator-dev`. Set `AYNI_ORCHESTRATOR_NAMESPACE` only when an isolated
namespace is intentional.

## Validate the workspace

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
pnpm install --frozen-lockfile
pnpm frontend:check
pnpm --filter @ayni-orchestrator/web exec playwright install chromium
pnpm test:e2e
```

`frontend:check` type-checks the generated Node bindings, server-only BFF, and
SvelteKit application; runs mapping, safe-error, correlation, and native gRPC
integration tests; builds the adapter-node runtime; and verifies native gRPC
code did not enter the browser bundle. `test:e2e` exercises the operator workflow,
keyboard/focus behavior, narrow view, reduced motion, theme switching, and
explicit destructive-action confirmation against a deterministic native-gRPC
fixture.

CI also installs the checksum-pinned Ayni release recorded by `.ayni.lock`,
builds and doctors the locked environment, runs the repository-complete check,
and retains its evidence. Exercise the same managed boundary locally with:

```sh
ayni env build --repo-root .
ayni env doctor --repo-root .
ayni check --config .ayni.toml
```

Render the core and optional observability deployments separately when changing
manifests:

```sh
scripts/local/render.sh local/control-plane:test local/worker:test local/web:test \
  >/tmp/ayni-orchestrator.yaml
scripts/local/render-observability.sh >/tmp/ayni-orchestrator-observability.yaml
```

Do not commit rendered output, logs, local credentials, or runtime artifacts.

## Validate the protobuf contract

The canonical product contract is under `proto/`. Install the locked local
TypeScript generator, validate the schema, reproduce the checked-in server
bindings, and compile both generated language surfaces with:

```sh
pnpm install --frozen-lockfile
scripts/ci/check-proto.sh
cargo test -p ayni-orchestrator-api
```

`buf format -w proto` formats source definitions. Do not hand-edit files under
`clients/typescript/src/generated`; run `buf generate` and review the resulting
diff. For an established contract, pass a base module to the check script to
run the additive-only compatibility check:

```sh
scripts/ci/check-proto.sh /path/to/base/checkout/proto
```

The product contract is native gRPC only. The retired `/v1` JSON product routes
and embedded control-plane UI are not served. `/livez`, `/readyz`, and `/metrics`
remain HTTP operational endpoints rather than alternate product APIs.

The local gRPC listener is native cleartext HTTP/2 on pod loopback. The Service
declares its target so `kubectl port-forward` can select it, but normal Service
traffic addresses the pod IP and cannot reach the loopback listener. Open a
host-loopback forward and exercise health, development reflection, and one
bounded read with:

```sh
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward --address 127.0.0.1 service/control-plane 15051:50051
grpcurl -plaintext -max-time 5 -d \
  '{"service":"ayni.orchestrator.v1.RepositoryService"}' \
  127.0.0.1:15051 grpc.health.v1.Health/Check
grpcurl -plaintext -max-time 5 127.0.0.1:15051 list
grpcurl -plaintext -max-time 10 -d '{}' 127.0.0.1:15051 \
  ayni.orchestrator.v1.RepositoryService/ListRepositories
```

Never forward the local listener to a non-loopback address. To develop or test
approved custom-client access, use the distinct opt-in TLS overlay and the
private descriptor workflow in [Custom-client gRPC access](custom-client-access.md).
That profile remains private and ClusterIP-only; it is not authorization for
public hosting.

## Configure local credentials

Copy the documented template to the gitignored `.env` file and set the GitHub
App ID, installation ID, OpenAI key, and base64-encoded GitHub App private key.
Base64 is transport encoding, not encryption.

```sh
cp .env.example .env
scripts/local/configure-development-secrets.sh
```

The bootstrap script parses only documented keys and does not source the file.
It writes Kubernetes Secrets for the local namespace. Never print those Secrets
as YAML or commit `.env`.

The GitHub App should be installed only on the development repository with:

- Metadata: read
- Contents: read and write
- Pull requests: read and write

Webhooks, organization permissions, Actions access, and user authorization are
not required by this profile.

## Build, deploy, and check

Use a clean committed checkout:

```sh
scripts/local/deploy.sh
scripts/local/check.sh
```

The deploy script:

1. verifies Docker Desktop capacity and the explicit Kubernetes context;
2. builds control-plane, worker, and SvelteKit web images labeled with the
   checkout revision and tags each with a content-identity suffix;
3. loads all three into Docker Desktop's Kubernetes node;
4. renders and applies the Kustomize overlay;
5. waits for rollout; and
6. verifies the running control-plane and web image identities and the worker
   image frozen into the control-plane Pod configuration.

It preserves SQLite, checkpoint, validation-result, and prepared-environment
volumes across updates. It may remove only obsolete project image tags after a
successful rollout; it does not prune shared Docker caches or volumes.

The check script opens separate loopback web, operational HTTP, and gRPC
port-forwards. It verifies `/livez`, `/readyz`, metrics, the server-rendered BFF
page through real native gRPC reads, gRPC health, and reflection. The current
readiness JSON field is named
`pilot_ready`; it means publication is configured and provider model discovery
has succeeded at least once in this process, not that an individual repository
has been prepared or the provider is currently reachable.

Remove stateless resources while preserving retained state with:

```sh
scripts/local/cleanup.sh
```

See [local operations](operations.md) before deleting a namespace, persistent
volume, or Docker data.

## Manual managed-check path

A standalone Job exercises the worker without the control-plane task API:

```sh
scripts/local/run-managed-check.sh
```

It fetches the pinned source revision, verifies the committed Ayni lock, runs
`env build`, `env doctor`, and an unscoped managed check through an isolated
Docker-in-Docker engine, then stores original artifacts and a digest-bound
manifest on `managed-check-evidence`.

The command refuses to replace an existing Job. Inspect logs and retained data
before deleting it. Quality failure, source/lock mismatch, and missing artifacts
remain distinct terminal results.

Exercise missing-artifact rejection with:

```sh
scripts/local/run-artifact-rejection-check.sh
```

The dedicated test Job removes `execution.json` after Ayni runs. The unchanged
worker must withhold success and report the missing artifact.

## Product workflow

The current profile accepts only the private `ayni-oss/sandbox` Rust repository
and Ayni `0.13.0`. Use the SvelteKit operator interface for enrollment, exact-revision
preparation, task submission, lifecycle actions, usage, and evidence. Approved
automation may use the native gRPC services documented under `proto/`; there is
no REST or JSON product API.

Repository preparation must be `ready` for the exact revision and lock. A task
freezes its repository, provider/model selection, source, lock, preparation,
request, acceptance criteria, change constraints, and deadline. Retries and
requested-input follow-ups create linked tasks rather than mutating prior state.

The coding Job has fixed checkout file and command tools, no Kubernetes token,
and no GitHub App private key. Independent validation writes retained evidence,
and publication uses only the validated candidate identity to reconcile one
deterministic branch and draft pull request. Backend application operations own
authorization and lifecycle meaning; clients must not infer transitions.

## Browser server boundary

Open the SvelteKit server only through loopback port-forwarding:

```sh
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward --address 127.0.0.1 service/web 18080:8080
open http://127.0.0.1:18080/
```

The deployed Svelte 5 operator application provides repository readiness, task
operation, lifecycle, usage, diagnostics, and structured evidence. Its server
loads and form actions call only the server-side BFF, which uses generated native
gRPC clients over pod loopback and returns explicit browser-safe DTOs. A bounded same-origin task update relay is available at
`/tasks/TASK_ID/updates`; it is a private browser implementation detail, not a
supported JSON API.

The control-plane HTTP listener serves only operational probes, Prometheus
metrics, and a bearer-protected internal publication callback. It does not serve
a browser or product JSON routes. The browser remains unauthenticated; anyone
who can reach its loopback forward can operate it.

Observability is optional and is not installed by `deploy.sh`. After the core
check passes, install and validate the disposable local stack explicitly:

```sh
scripts/local/deploy-observability.sh
scripts/local/check-observability.sh
```

This adds pinned Prometheus, kube-state-metrics, and Grafana images. Prometheus
scrapes the application endpoint, namespaced Kubernetes object state, and the
Docker Desktop kubelet/cAdvisor through a service account limited to node
discovery and the metric-only `nodes/metrics` subresource. It is not granted
`nodes/proxy` or access to Secrets, pod specifications, or logs. Open the
provisioned dashboard through loopback only:

```sh
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward service/grafana 13000:3000
open http://127.0.0.1:13000/d/ayni-orchestrator-tasks-resources
```

Prometheus retains at most 48 hours or 900 MB in ephemeral storage. Grafana
anonymous access is Viewer-only and acceptable solely within this trusted local
boundary. Remove only the optional stack with
`scripts/local/teardown-observability.sh`; this does not touch product state.
See [local operations](operations.md) for metric semantics, capacity, and
troubleshooting.
