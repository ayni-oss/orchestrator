# Local development

The supported local profile runs the control plane and workers on Docker Desktop
Kubernetes. It includes durable task orchestration, repository preparation,
bounded coding, independent candidate validation, draft-PR publication, and the
browser interface.

## Prerequisites

- Rust 1.94.0, pinned by `rust-toolchain.toml`
- Docker Desktop with Kubernetes enabled
- a reachable `docker-desktop` Kubernetes context
- `kubectl`, Docker, Git, and `curl`

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
```

Render the deployment separately when changing manifests:

```sh
scripts/local/render.sh >/tmp/ayni-orchestrator.yaml
```

Do not commit rendered output, logs, local credentials, or runtime artifacts.

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
2. builds control-plane and worker images labeled with the checkout revision;
3. loads them into Docker Desktop's Kubernetes node;
4. renders and applies the Kustomize overlay;
5. waits for rollout; and
6. verifies the running control-plane image identity.

It preserves SQLite, checkpoint, validation-result, and prepared-environment
volumes across updates. It may remove only obsolete project image tags after a
successful rollout; it does not prune shared Docker caches or volumes.

The check script opens a loopback port-forward, verifies `/livez`, `/readyz`, the
browser marker, and list APIs. The current readiness JSON field is named
`pilot_ready`; it means publication configuration is available, not that an
individual repository has been prepared.

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

## Durable managed-task API

Submit immutable managed-check inputs to `POST /v1/tasks` with an idempotency
key, then inspect `GET /v1/tasks/{task_id}`. The reconciler uses a deterministic
Job name and observes the same Job after restart. Cancellation records durable
intent before deleting the owned Job. A late report cannot revive a cancelled
attempt.

Retry is explicit and idempotent. Only failed or interrupted attempts can be
retried. Each task retains the maximum-attempt bound that applied at submission.
The SQLite storage model supports one control-plane replica only.

## Development-task API

The current profile accepts only the private `ayni-oss/sandbox` Rust repository
and Ayni `0.13.0`. Use the browser for enrollment and preparation, or configure:

- `POST /v1/repository-bindings` with repository identity, GitHub App
  installation, base branch, and lowercase SHA-256 of `.ayni.lock`;
- `POST /v1/agent-profiles` with provider `openai`, model, and Kubernetes Secret
  reference; and
- `POST /v1/development-tasks` with a unique idempotency key, binding/profile
  IDs, exact 40-hex start revision, request, acceptance criteria, and optional
  `deadline_seconds` from 1 through 86400.

Repository preparation must be `ready` for those exact inputs. A task freezes
its repository, profile, source, lock, preparation, request, acceptance criteria,
and deadline. Reusing an idempotency key with different content is rejected.

The coding Job has fixed checkout file and command tools, no Kubernetes token,
and no GitHub App private key. It stores a local commit and Git bundle on
`development-checkpoints`. Validation mounts the bundle read-only, reconstructs
the candidate, loads the retained Ayni environment in a fresh isolated engine,
and writes artifacts to `managed-check-evidence`. Publication uses only the
validated candidate identity and reconciles one deterministic branch and draft
pull request.

Candidate changes to `.ayni.toml` are rejected. Changes to `.ayni.lock`, Rust
dependency manifests, or the pinned toolchain return the explicit
`dependency_change_unsupported` result. Related future requirements are tracked
in the [Orchestrator Project](https://github.com/orgs/ayni-oss/projects/2).

## Browser workflow

Open the Service only through loopback port-forwarding:

```sh
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward service/control-plane 18080:8080
open http://127.0.0.1:18080/
```

The browser exposes repository readiness, preparation, task submission, phase
refresh, cancellation, linked retry, requested-input follow-up, draft PR links,
and validated results. It is not authenticated; anyone who can reach the Service
can operate it.
