# Local development

M2 deploys a persistent control plane that accepts durable managed-check tasks
and reconciles attempt-owned Jobs. It does not yet run an agent or create draft
pull requests. Its readiness response reports `managed_execution: true` and
`durable_orchestration: true`.

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
