# Local development

> **Clean-v1 transition.** The controller now initializes and serves the
> versioned clean-v1 durable resource baseline. Profile-specific automation,
> runtime execution, and managed assessment remain follow-on delivery; do not
> interpret retired pilot preparation, local image archives, or `pilot_ready`
> as current v1 behavior.

The supported target topology is Docker Desktop Kubernetes with explicit
`docker-desktop` context, one trusted maintainer team, and loopback-only access.
`dev/ctl` remains the local command boundary. It must never fall back to another
cluster, reset retained data, or print local credentials.

## Clean-v1 local contract

A clean installation starts with the replacement database baseline; it does not
migrate the pilot SQLite schema, preparation directories, checkpoints, or
validation manifests. Preserve any pilot data outside the deployment if needed,
then use the guarded purge path before installing v1. Do not add a legacy-read
or migration flag.

The target deployment requires configuration for the GitHub App, enrolled GHCR
coordinates, supported platform(s), Ayni release/checksum, workflow-template
version, reconciliation interval/jitter, deadlines/concurrency, artifact bounds,
and readiness policy. Credentials are supplied only through ignored local
configuration and scoped Kubernetes Secrets; never commit, print, or copy them
into task evidence.

A GitHub App installation needs only the permissions documented and enforced by
the [GitHub and GHCR security boundary](github-security.md): repository/default-branch
read, owned workflow/setup-PR creation, Actions/check observation or dispatch where
needed, Packages read, and bounded comment/PR operations. Webhooks, if enabled later,
are authenticated reconciliation hints rather than an execution or authorization path.
The default-branch environment workflow has package-write only for its trusted
build job; PR code never receives package-write, controller, or model
credentials.

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
4. Run `dev/ctl check --expect-empty`. The readiness response must report `clean_schema_version: 1`;
   initial resource lists must be empty, and logs must not report a retired
   schema or corrupt protobuf payload.

If startup rejects an unexpected old or corrupt volume, preserve it for
investigation, repeat the guarded purge, and redeploy. Never rename pilot tables,
change the schema marker, or use a compatibility flag to bypass the rejection.

## Target developer workflow

1. Enroll a GitHub repository as a **repository connection** and create an
   **Ayni Managed** profile. It must contain committed `.ayni.toml` and
   `.ayni.lock`; the Ayni executable is provisioned, not vendored.
2. Review the idempotent pipelines setup PR. The profile is
   `setup_pr_open` until it merges; no default-branch mutation occurs directly.
3. Observe trusted default-branch automation validate the lock and publish or
   verify `ghcr.io/<owner>/<repository>/ayni-environment:lock-<full-64-hex-lock-fingerprint>`.
   The controller independently resolves and verifies its digest, labels, owner,
   and platform before `ready`.
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

The usual workspace checks remain required for documentation and implementation
changes:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
pnpm install --frozen-lockfile
pnpm frontend:check
scripts/ci/check-proto.sh
scripts/ci/check-helm.sh
```

When a delivery issue changes deployment behavior, render through the explicit
context and exercise its disposable Docker Desktop scenario. #68 owns the
recorded end-to-end gate: exactly one setup PR; verified GHCR artifact; coding
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
