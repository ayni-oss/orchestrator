# Local development

The current pilot profile combines persistent managed-check orchestration,
API-driven coding, independent validation, draft-PR publication, a browser
interface, repository readiness, content-addressed environment preparation and
reuse, task actions, and evidence views. `/readyz` reports whether the
publication and preparation subsystem is configured; repository-specific access
is verified by preparation.

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

## Build, deploy, and check

From a clean committed checkout, configure the local development Secrets as
described under [Development request to draft PR](#development-request-to-draft-pr),
then run:

```sh
scripts/local/deploy.sh
scripts/local/check.sh
```

The local scripts expose one current profile rather than historical delivery
modes. `deploy.sh` and `check.sh` reject positional arguments; set
`AYNI_ORCHESTRATOR_NAMESPACE` to override the namespace.

`deploy.sh` creates control-plane and worker image references from the checked-out
commit, verifies their Docker revision labels, and imports them into Docker
Desktop's single kind control-plane node before rendering the overlay. It
refuses to build below the profile's free-space floor, uses named BuildKit cache
mounts rather than retaining a complete Cargo target layer per source revision,
and removes only obsolete project image tags after a successful rollout. It then
waits for the Deployment and records the control-plane image reference and image
ID used by the pod. The image imports and final pod image-identity check prove
this local delivery path; a host Docker build alone is not treated as deployment
evidence. The control-plane state PVC is retained across Deployment replacement.
The shared managed-check evidence PVC is retained separately and is never removed
by normal service cleanup.

`check.sh` uses loopback port-forwarding, requires pilot readiness, fetches the
HTML marker, and checks the repository and task list APIs. It does not execute
browser JavaScript or replace the browser walkthrough.
To remove only the overlay's service resources while preserving the namespace,
run `scripts/local/cleanup.sh`.

For each deployment, record the commit, image identity, Docker Desktop,
Kubernetes, `kubectl`, and renderer versions in the pull request. Generated
logs, port-forward output, and secrets must not be committed.

## Manual managed check

The standalone managed check is intentionally a manually submitted Job, not a
control-plane feature. It fetches the pinned public Ayni release commit, verifies its committed lock,
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

## Durable task API

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

## Development request to draft PR

Development tasks support only the private `ayni-oss/sandbox` Rust workspace.
The committed `.ayni.lock` in that repository must match the repository binding. Initialize
the sandbox once with the Rust workspace contract and deliberately wrong
`examples/retry-backoff/main.rs` fixture; it must contain the Ayni contract,
generated lock, Rust workspace files, and fixture test script. Running
`scripts/local/check-retry-backoff-fixture.sh` in the sandbox before the task
must fail at attempt 3. Its expected cases are `1→2`, `2→4`, `3→8`, and `6→60` seconds. This
fixture is not the control plane's own retry schedule.

Generate or refresh the lock with the pinned Ayni **0.13.0** executable from
[`ayni-oss/ayni`](https://github.com/ayni-oss/ayni/releases/tag/ayni-v0.13.0),
the same release used by the worker image. Verify that the generated lock uses
`ghcr.io/ayni-oss/ayni-provisioning`; the deprecated `gdurandvadas/ayni`
release and registry are not supported. The 0.13.0 lock names the pinned
`ghcr.io/ayni-oss/ayni-provisioning` image used in the completed deployed
proof. A missing pinned digest is an external-blocked result, never a reason to
substitute another image.

Immediately before the live exercise, create a GitHub App and install it only
on `ayni-oss/sandbox`. Grant Metadata **read**, Contents **write**, and Pull
requests **write**. Do not enable webhooks, organization permissions, workflow
access, or a user authorization flow. The App ID and installation ID are not
secrets; the App private key and OpenAI API key are. Keep credentials out of Git,
terminal arguments, and transcripts. The project does not require a particular
password manager or secret-store vendor.

For local development, copy the public `.env.example` to the gitignored `.env`.
Set the App ID, App installation ID, OpenAI key, and the downloaded GitHub App
PEM encoded as one base64 line. Base64 makes the multiline PEM convenient for a
dotenv file; it is not a security mechanism. The bootstrap script parses only
the documented keys, does not source or evaluate the file, and removes its
temporary decoded files on exit. It creates or updates the GitHub App and
OpenAI Secrets, and creates the internal publisher Secret only if it is absent:

```sh
cp .env.example .env
scripts/local/configure-development-secrets.sh
```

The App ID, PEM, and OpenAI key become Kubernetes Secret keys. The App
installation ID remains outside the Secret and is entered through the binding
API. You do not create or manage the internal publisher token. Do not commit
`.env` or print the Secrets as YAML. Kubernetes Secrets are separate from Git
but are not automatically encrypted at rest; treat Docker Desktop cluster data
and backups as sensitive.

From a clean committed checkout, `scripts/local/deploy.sh` builds and loads both
images, renders the current overlay, and replaces the local control plane. It
preserves the SQLite, evidence, checkpoint, and preparation PVCs. Check
`scripts/local/check.sh`, then use loopback `kubectl --context docker-desktop
-n ayni-orchestrator-dev port-forward svc/control-plane 8080:8080` for the API.
No endpoint should be publicly exposed.

Configure one binding via `POST /v1/repository-bindings` with JSON fields
`id`, `owner: "ayni-oss"`, `repository: "sandbox"`, `installation_id`,
`base_branch: "main"`, and `lock_sha256` (lowercase SHA-256 hex of the sandbox
`.ayni.lock`). Configure `POST /v1/agent-profiles` with `id`,
`provider: "openai"`, `model`, and `secret_ref: "openai-api"`. Submit
`POST /v1/development-tasks` with a unique `idempotency_key`, the binding and
profile IDs, the exact 40-hex `start_revision`, a `request` to correct the
fixture to `min(60, 2 × 2^(attempt−1))`, and `acceptance_criteria` containing
the four empirical cases. Repeating a submission key with different content
is rejected. Inspect `GET /v1/development-tasks/{task_id}`. Cancellation is
`POST /v1/development-tasks/{task_id}/cancel` with a new idempotency key.
Configured non-OpenAI providers return `unsupported` without a Job.

The coding Job has only fixed checkout file/command tools, no Kubernetes
service-account token, and no GitHub App key. Its frozen candidate bundle is
retained on `development-checkpoints`. The independent validation Job mounts that bundle
read-only, writes original Ayni artifacts and a manifest to
`managed-check-evidence`, and refuses to publish an unverified candidate. A
quality failure or protected policy edit remains a failed task; test-file edits
are disclosed in the task and draft PR. The PR-author Job has only the
structured publish capability; the control plane owns GitHub token minting and
deterministic branch/PR reconciliation. The draft PR targets the sandbox's
`main` branch; neither it nor its task branch is merged by this workflow.

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

## Browser pilot and prepared environments

Configure credentials, deploy, and validate the current profile with:

```sh
scripts/local/configure-development-secrets.sh
scripts/local/deploy.sh
scripts/local/check.sh
```

Use only a loopback port-forward and open `/`. Enroll the sandbox and prepare an
exact revision before submitting a task; submissions fail closed when no matching
ready preparation exists. Preparation retains an Ayni-built image archive and
build record on `prepared-environments`; independent validation verifies and
loads that exact image rather than rebuilding. Repeating compatible preparation
records warm reuse and changed inputs require a new cold build.

The browser lists repository readiness and tasks, follows phases, and exposes
cancel, linked retry, requested-input follow-up, draft PR, and evidence actions.
Dependency-manifest changes return an explicit unsupported result in this pilot.
See [pilot operations](pilot-operations.md) for access boundaries, resource and
retention limits, install/update/recovery procedures, and failure triage.
