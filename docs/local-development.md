# Local development

> **Clean-v1 transition.** The controller now initializes and serves the
> versioned clean-v1 durable resource baseline. Profile-specific automation,
> runtime execution, and managed assessment remain follow-on delivery; do not
> interpret retired pilot preparation, local image archives, or `pilot_ready`
> as current v1 behavior.

The supported target topology is Docker Desktop Kubernetes with explicit
`docker-desktop` context, one trusted maintainer team, and loopback-only access.
`dev/ctl deploy` enables managed runtime execution, builds the worker for the
workflow's `linux/amd64` platform, and pins its loaded manifest digest automatically.
Image loading selects the concrete platform manifest so Apple Silicon nodes can
run the worker under emulation without trying to fetch a local tag from Docker Hub.
Connected repositories inherit the local `runtime-pull` Secret reference; no
per-repository Helm values are needed. Helm plugins are disabled for this command.
The reusable production chart remains opt-in.

`dev/ctl` remains the local command boundary. It must never fall back to another
cluster, reset retained data, or print local credentials.

## Clean-v1 local contract

A clean installation starts with the replacement database baseline; it does not
migrate the pilot SQLite schema, preparation directories, checkpoints, or
validation manifests. Preserve any pilot data outside the deployment if needed,
then use the guarded purge path before installing v1. Do not add a legacy-read
or migration flag.

The target deployment requires configuration for the supported registry,
profile source (registry or dedicated build Job), supported platform(s), Ayni
release/checksum, reconciliation interval/jitter, deadlines/concurrency,
artifact bounds, and readiness policy. GitHub App configuration is needed only
when repository assessment automation is enabled. Credentials are supplied only
through ignored local configuration and scoped Kubernetes Secrets; never commit,
print, or copy them into task evidence.

A GitHub App installation, when assessment automation is enabled, needs only
the permissions documented and enforced by the
[GitHub security boundary](github-security.md): repository/default-branch read,
owned workflow/setup-PR creation, Actions/check observation or dispatch where
needed, and bounded comment/PR operations. Webhooks, if enabled later, are
authenticated reconciliation hints rather than an execution or authorization
path. Registry pull and build-push credentials are Kubernetes-scoped; PR code
never receives them, controller credentials, or model credentials.

## Fresh project-schema installations

The project baseline is schema version 3. Versions 1 and 2 are rejected without
migration. Deploy into an isolated namespace with new storage; preserve existing
state. The guarded purge procedure below is destructive and requires a separate
explicit decision when retained data exists.

## One-time pilot reset

The controller deliberately refuses a database containing retired pilot tables;
it never upgrades or reads that state. Before the first clean-v1 deployment:

1. Run `dev/ctl status` and record the configured `ayni-orchestrator-*`
   namespace. If pilot data must be retained, stop writers with `dev/ctl
   cleanup`, then copy or snapshot the retained volume using an approved local
   backup mechanism. Store the backup outside the namespace with restricted
   permissions. It is archival only and must never be restored into clean v1.
2. Ensure the private environment file still exists outside Kubernetes. Run
   `dev/ctl purge <exact-namespace>`, substituting the namespace printed by
   `dev/ctl status`. This guarded command accepts only the configured, labeled
   local namespace and permanently removes its Secrets, Jobs, and volumes.
3. Recreate scoped Secrets with `dev/ctl secrets`, then install a fresh baseline
   with `dev/ctl deploy`. Do not copy the pilot SQLite file, preparation
   directories, checkpoints, archives, or manifests into the new volume.
4. Run `dev/ctl check --expect-empty`. The readiness response must report `clean_schema_version: 3`;
   initial resource lists must be empty, and logs must not report a retired
   schema or corrupt protobuf payload.

If startup rejects an unexpected old or corrupt volume, preserve it for
investigation, repeat the guarded purge, and redeploy. Never rename pilot tables,
change the schema marker, or use a compatibility flag to bypass the rejection.

## Target developer workflow

1. Enroll a GitHub repository as a **repository connection** and create an
   **Ayni Managed** profile. It must contain committed `.ayni.toml` and
   `.ayni.lock`; the Ayni executable is provisioned, not vendored.
2. Set up the runtime. Matching workflows on the default branch are reused. If
   workflows are missing or different, review the setup PR; the profile stays
   `setup_pr_open` until it merges.
3. The controller checks the repository's expected GHCR image automatically and
   verifies its identity, certificate, and platform before marking it `ready`.
   If the image is missing, run the environment workflow and retry reconciliation.
4. Submit coding only against a ready profile. The coding Job uses the frozen
   `image@sha256:...` digest, writable checkout, bounded scratch, and scoped
   credentials. It does not build an environment or mount a Docker socket,
   controller database, service-account token, or another task's state.
5. Treat plain `ayni check` in the coding container as feedback. Publish a
   candidate as a draft PR, then let managed PR automation run exact-head
   `ayni check` and retain digest-verified assessment evidence.

A source-only default-branch revision reuses/validates its lock artifact. A
lock revision enters `updating`; new tasks wait for its verified artifact while
old in-flight tasks retain their frozen digest. `Custom Runtime` is visible as a
profile variant but is not operational until #66.

## Validation commands

### Testing an unreleased Ayni fix

Use the sibling Ayni checkout for the local repair loop; publishing a release is
not a prerequisite. From this repository, build the checkout CLI, stage tracked
working-tree files into a clean temporary directory, and run the same image-build
and container-check sequence as CI:

```sh
(cd ../ayni && cargo build -p ayni-cli)
ayni_bin="$(cd ../ayni && pwd)/target/debug/ayni"
check_root="$(mktemp -d)"
git ls-files -z | rsync -a --from0 --files-from=- ./ "$check_root/"
"$ayni_bin" env build --repo-root "$check_root" \
  --tag local/orchestrator-ayni-checkout \
  --executor-image "$AYNI_EXECUTOR_IMAGE"
docker run --rm --user "$(id -u):$(id -g)" \
  --env HOME=/tmp/home --env XDG_CACHE_HOME=/tmp/cache \
  --env MISE_CACHE_DIR=/tmp/mise --env CARGO_HOME=/tmp/cargo \
  --env CARGO_TARGET_DIR=/tmp/target \
  --mount "type=bind,source=$check_root,target=/workspace" \
  --workdir /workspace local/orchestrator-ayni-checkout \
  ayni check --config .ayni.toml --output markdown
```

Set `AYNI_EXECUTOR_IMAGE` to the immutable executor reference matching the
checkout version. A released executor is sufficient for changes solely to the
host-side image recipe; changes to code executed inside the container require a
Linux executor built from that checkout too. Use Ayni's candidate-image tooling
and a local registry for that case, not a public release. If the checkout changes
lock inputs or version, regenerate the lock in the temporary snapshot with its
CLI and the existing pinned provisioning base. Never hand-edit the fingerprint.
The snapshot and its `.ayni/last/signals.json` remain available for inspection.

Both image construction and the in-container check must pass before claiming
managed compatibility. A host test pass alone is insufficient. Keep private
source, logs, and fixtures out of public Ayni contributions; upstream regression
tests must stand alone.

### Repository checks

The usual workspace checks remain required for documentation and implementation
changes:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
pnpm install --frozen-lockfile
ayni check --config "$PWD/.ayni.toml"
pnpm frontend:check
PLAYWRIGHT_USE_EXISTING_BUILD=1 pnpm test:e2e
scripts/ci/check-proto.sh
scripts/ci/check-helm.sh
```

Ayni owns Rust and Node unit-test, size, complexity, and package-dependency
signals. The Node roots are `apps/web` and `packages/bff`; both emit real Vitest
JSON evidence. Workspace membership is declared consistently in `package.json`
and `pnpm-workspace.yaml` so Ayni resolves the shared runtime and lockfile.
`frontend:check` is only the complementary TypeScript/Svelte compilation,
production build, and browser-bundle boundary check. Browser tests reuse that
build; a standalone `pnpm test:e2e` still builds automatically. These complementary
checks remain local commands, not a standalone frontend CI job. The Ayni job is
the Rust/Node quality gate; it does not claim browser-test or build validation.

Run Ayni before a production build in a clean checkout, as CI does. Ayni 0.14.2
does not apply the Node size exclusions to its ESLint complexity scan, so an
existing `.svelte-kit`/`build` output tree is not a valid source-only complexity
baseline. Svelte component script complexity is not measured by this adapter;
Svelte compilation and browser checks remain explicit. Coverage stays disabled,
matching the existing repository policy.

When a delivery issue changes deployment behavior, render through the explicit
context and exercise its disposable Docker Desktop scenario. #68 owns the
recorded end-to-end gate: verified configured-registry artifact; coding
from its digest on a clean node/cache; draft PR; exact-head assessment and one
summary comment; source-only no-op verification; lock update; and recovery
coverage. A host check, mocked registry, or locally cached image alone is not
that evidence.

## Access boundary

Product gRPC stays native and pod-loopback by default. Open web, gRPC, metrics,
and Grafana only through `kubectl --context docker-desktop --address 127.0.0.1
port-forward`; no Ingress, NodePort, public image publication, or public hosting
is authorized. The SvelteKit BFF is server-only and not a second API. See
[protocol](../proto/README.md), [architecture](architecture.md), and
[operations](operations.md) for the target resource and recovery contract.

The local Grafana chart bounds Go scheduler concurrency with `GOMAXPROCS=1`
under its existing sub-core CPU quota. This prevents excess scheduler parallelism
from starving health probes on Docker Desktop; CPU and memory limits remain
explicit in the local chart.
