# Local development

M2 deploys a persistent control plane that accepts durable managed-check tasks
and reconciles attempt-owned Jobs. The optional M3 overlay adds an API-driven
coding agent, independent managed validation, and draft-PR publication. It has
no browser UI. Its readiness response does not establish that M3 secrets or
external services are available.

## Prerequisites

- Rust 1.94.0 (pinned in `rust-toolchain.toml`)
- Docker Desktop with Kubernetes enabled and a reachable `docker-desktop`
  context
- `kubectl`, Docker, and `curl`

The scripts always name `docker-desktop` explicitly. They stop if that context
is unavailable and never use the current context. They deploy into
`ayni-orchestrator-dev`; do not use the scripts to reset a Kubernetes cluster.

## Validate the workspace

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
```

## M2: build, deploy, and check

From a clean committed checkout, run:

```sh
scripts/local/deploy.sh
scripts/local/check.sh
```

`deploy.sh` creates control-plane and worker image references from the checked-out
commit, verifies their Docker revision labels, and imports them into Docker
Desktop's single kind control-plane node before rendering the overlay. It then
waits for the Deployment and records the control-plane image reference and image
ID used by the pod. The image imports and final pod image-identity check prove
this local delivery path; a host Docker build alone is not treated as deployment
evidence. The control-plane state PVC is retained across Deployment replacement.
The shared managed-check evidence PVC is retained separately and is never removed
by normal service cleanup.

`check.sh` uses loopback port-forwarding and checks both `/livez` and `/readyz`.
To remove only the overlay's service resources while preserving the namespace,
run `scripts/local/cleanup.sh`.

For each deployment, record the commit, image identity, Docker Desktop,
Kubernetes, `kubectl`, and renderer versions in the pull request. Generated
logs, port-forward output, and secrets must not be committed.

## M1: manual managed check

M1 is intentionally a manually submitted Job, not a control-plane feature. It
fetches the pinned public Ayni release commit, verifies its committed lock,
then runs `env build`, `env doctor`, and an unscoped managed `check` through an
attempt-local Docker-in-Docker engine. It writes command logs, original Ayni
artifacts, and a hash-bound manifest to the `managed-check-evidence` PVC.

The Docker Desktop cluster must have at least 5 CPUs, 10GiB memory, and 20GiB
ephemeral storage available: the pinned Ayni lock itself requests 4 CPUs and
8GiB memory from the isolated engine. The Job uses a privileged engine sidecar;
the worker remains non-root and receives no Kubernetes service-account token or
host Docker socket. Its Buildx client configuration uses an attempt-local
`/tmp` scratch volume. Ayni temporary bind inputs use the shared
`/workspace` attempt volume so the isolated engine, rather than the worker
container root filesystem, can resolve them.

From a clean committed checkout, start one attempt with:

```sh
scripts/local/run-managed-check.sh
```

The command refuses to replace an existing Job. Inspect its logs and the
retained PVC first; deletion of an existing Job or evidence remains an explicit
orchestrator action. A quality failure leaves validated artifacts and causes the
Job to fail. Missing or mismatched artifacts are infrastructure failures and
are never recorded as a successful managed result.

To exercise the final-artifact gate, run:

```sh
scripts/local/run-artifact-rejection-check.sh
```

It runs the normal managed check in the selected topology, then a test-only
wrapper removes `execution.json` before the worker's existing artifact-binding
step. The Job must fail with `missing execution artifact`; a completed Job or a
different failure does not satisfy the gate. The wrapper is mounted only by this
dedicated test Job and does not alter the worker image or normal managed-check
manifest. Inspect retained command logs before explicitly deleting its Job.

## M2: durable task API

After `scripts/local/deploy.sh`, submit the same immutable managed-check inputs
through `POST /v1/tasks` with an idempotency key. Inspect `GET /v1/tasks/{task_id}`.
The reconciler creates a deterministic attempt Job; repeated reconciliation after
a restart finds that Job rather than scheduling another one. Cancellation marks
the task durable first, then deletes only that attempt Job. A report arriving
after cancellation is retained for audit but cannot revive the attempt.

The first local design intentionally runs one control-plane replica because
SQLite uses a single retained read-write PVC. Do not scale it horizontally or
mount the state PVC into a worker. Retry requires an explicit idempotency key and
only failed or interrupted attempts can create a new attempt. Each task retains
the configured `ORCHESTRATOR_MAX_ATTEMPTS` bound at submission (three by default;
the local profile accepts one through ten), so a later deployment change does not
expand its retries.

## M3: API request to draft PR

M3 supports only the private `ayni-oss/orchestrator` Rust workspace. The
committed `.ayni.lock` must match the repository binding. The deliberately wrong
fixture is `examples/m3-retry-backoff/main.rs`; running
`scripts/local/check-m3-fixture.sh` before the task must fail at attempt 3.
Its expected cases are `1→2`, `2→4`, `3→8`, and `6→60` seconds. This fixture is
not the control plane's own retry schedule.

Generate or refresh the lock with the pinned Ayni **0.12.1** executable used by
the worker image. A lock made with the host's newer Ayni 0.13.0 is incompatible
with this deployment and may reference a different provisioning registry.

Immediately before the live exercise, create a GitHub App and install it only
on `ayni-oss/orchestrator`. Grant Metadata **read**, Contents **write**, and Pull
requests **write**. Do not enable webhooks, organization permissions, workflow
access, or a user authorization flow. Keep the App private key and OpenAI key
outside Git and terminal transcripts. Create these Kubernetes Secrets in
`ayni-orchestrator-dev`: `m3-github-app` with `app-id` and `private-key.pem`,
`m3-publisher` with a randomly generated `token` of at least 32 characters,
and `m3-openai` with `api-key`. The App installation ID is entered through the
binding API, not placed in a Secret. Use the explicit `docker-desktop` context.

From a clean committed checkout, `scripts/local/deploy.sh m3` builds and loads
both images, renders the M3 overlay, and replaces the local control plane. It
preserves M2 SQLite and evidence PVCs and adds the `m3-checkpoints` PVC. Check
`scripts/local/check.sh`, then use loopback `kubectl --context docker-desktop
-n ayni-orchestrator-dev port-forward svc/control-plane 8080:8080` for the API.
No endpoint should be publicly exposed.

Configure one binding via `POST /v1/repository-bindings` with JSON fields
`id`, `owner: "ayni-oss"`, `repository: "orchestrator"`, `installation_id`,
`base_branch: "feat/request-to-draft-pr"`, and `lock_sha256` (lowercase SHA-256
hex of `.ayni.lock`). Configure `POST /v1/agent-profiles` with `id`,
`provider: "openai"`, `model`, and `secret_ref: "m3-openai"`. Submit
`POST /v1/development-tasks` with a unique `idempotency_key`, the binding and
profile IDs, the exact 40-hex `start_revision`, a `request` to correct the
fixture to `min(60, 2 × 2^(attempt−1))`, and `acceptance_criteria` containing
the four empirical cases. Repeating a submission key with different content
is rejected. Inspect `GET /v1/development-tasks/{task_id}`. Cancellation is
`POST /v1/development-tasks/{task_id}/cancel` with a new idempotency key.
Configured non-OpenAI providers return `unsupported` without a Job.

The coding Job has only fixed checkout file/command tools, no Kubernetes
service-account token, and no GitHub App key. Its frozen candidate bundle is
retained on `m3-checkpoints`. The independent validation Job mounts that bundle
read-only, writes original Ayni artifacts and a manifest to
`managed-check-evidence`, and refuses to publish an unverified candidate. A
quality failure or protected policy edit remains a failed task; test-file edits
are disclosed in the task and draft PR. The PR-author Job has only the
structured publish capability; the control plane owns GitHub token minting and
deterministic branch/PR reconciliation. The draft PR targets the M3 delivery
branch; neither PR is merged by this workflow.

For the completion record, retain the deployed commit, image IDs, Docker and
Kubernetes versions, rendered overlay, task and candidate IDs, evidence
manifest digest, draft PR URL, failure and cancellation task IDs, and the
result of restart/duplicate-action checks. A failed or unavailable external
service must be reported as blocked; local tests cannot substitute for this
live exercise. Checkpoints, SQLite state, and evidence PVCs are retained by
normal Job cleanup. Do not delete these PVCs merely to repeat a task.

On 2026-09-24, a Docker Desktop deployment of code commit `6dc4210` built and
loaded both images but the control plane could not initialize SQLite: the
Docker Desktop VM filesystem had 0 available space. The node reported no
`DiskPressure` despite the full filesystem. Before retrying deployment, inspect
VM and Docker usage and arrange capacity or explicitly approved cleanup. Do not
prune shared Docker volumes or caches as an implicit part of these scripts.
