# Local operations

This runbook is the operating contract for the supported Docker Desktop
profile: one trusted maintainer team, one local cluster, and the allowlisted
`ayni-oss/sandbox` repository. It is not a multi-tenant service contract.

## Access boundary

The control plane has no end-user authentication in this profile. The Service is
`ClusterIP`; do not expose it with an Ingress, `NodePort`, or a non-loopback
forward. Open the product only through:

```bash
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward service/control-plane 18080:8080
open http://127.0.0.1:18080/
```

Anyone able to reach the Service can operate the product. Kubernetes access,
Docker Desktop host access, the GitHub App installation, and the configured
model key are therefore privileged. The internal publication endpoint also
requires its dedicated bearer token. Neither GitHub nor model credentials are
returned by the API or UI.

## Prerequisites and capacity

- Docker Desktop Kubernetes, with current context exactly `docker-desktop`.
- `kubectl`, Docker, Git, Rust, and `curl`.
- At least 6 CPUs, 14 GiB memory, and 30 GiB free disk assigned to Docker
  Desktop. One managed validation/preparation engine requests 4 CPUs and 9 GiB.
- A GitHub App installed on the enrolled repository with Contents and Pull
  requests read/write permissions.
- A committed `.ayni.toml` and `.ayni.lock`; the worker uses Ayni `0.13.0`.
- The local credential file described in `.env.example`. Never commit it.

Storage claims are 1 GiB for SQLite state, 2 GiB each for checkpoints and
validation evidence, and 10 GiB for prepared image archives. The deploy script
requires at least 12 GiB free before an image build/import, then removes only
obsolete `local/ayni-orchestrator-*` image tags not referenced by a pod. Rust
Cargo registry and target data use named BuildKit cache mounts so source changes
do not create a new multi-gigabyte immutable builder layer each time. This does
not prune shared cache or Docker volumes. Docker Desktop's
single node permits the selected `ReadWriteOnce` sharing pattern. Do not copy
this topology to a multi-node cluster.

Current profile limits are one repository allowlist, 24 coding-agent turns, 4 PR
summary turns, one-hour preparation/validation Job deadlines, 5 GiB checkout
workspaces, 20 GiB isolated engine storage, and no automatic deletion of
retained state. Kubernetes capacity naturally serializes the 4 CPU/9 GiB heavy
Jobs on the recommended desktop allocation. Maintainers should submit one
change at a time.

## Clean installation

From a clean Docker Desktop Kubernetes namespace:

```bash
cp .env.example .env
# Fill values locally; do not print the file.
scripts/local/configure-development-secrets.sh
scripts/local/deploy.sh
scripts/local/check.sh
```

The check requires `pilot_ready: true`, fetches the HTML marker and repository
and task list APIs, and does not execute browser JavaScript. The first install
creates four retained PVCs. Re-running the install does not replace them.

To reproduce a clean installation without disturbing the normal development namespace,
use an isolated namespace and local port, then remove that namespace after
recording the result:

```bash
export AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-clean-check
export AYNI_ORCHESTRATOR_LOCAL_PORT=18081
scripts/local/configure-development-secrets.sh
scripts/local/deploy.sh
scripts/local/check.sh
kubectl --context docker-desktop delete namespace "$AYNI_ORCHESTRATOR_NAMESPACE"
unset AYNI_ORCHESTRATOR_NAMESPACE AYNI_ORCHESTRATOR_LOCAL_PORT
```

Namespace deletion is intentionally explicit because it deletes that isolated
installation's Secrets, Jobs, and retained claims. Never point this command at
the normal development namespace.

Use the UI to save repository enrollment. Configure an agent profile once with
the API if one is not already present:

```bash
curl --fail -H 'content-type: application/json' \
  --data '{"id":"openai-gpt5","provider":"openai","model":"gpt-5","secret_ref":"openai-api"}' \
  http://127.0.0.1:18080/v1/agent-profiles
```

The `secret_ref` is a Kubernetes Secret name, not secret material.

## Repository readiness and preparation

Enrollment stores repository identity, GitHub App installation, base branch,
and the SHA-256 of the committed Ayni lock. Preparing an exact 40-character Git
revision then:

1. mints short-lived installation access and checks out only that revision;
2. verifies `.ayni.toml`, `.ayni.lock`, and the enrolled lock digest;
3. builds and doctors the managed Ayni environment in an isolated engine;
4. retains an immutable image archive, Ayni build record, and digest manifest;
5. records cold-build duration and the content-addressed preparation identity.

A repeated request with the same binding, revision, lock, and Ayni recipe is a
warm reuse and increments `reuse_count` without scheduling another build.
Changed revision or lock creates a replacement identity. A failed replacement
is never allowed to fall back to an older preparation; fix access or inputs and
press Prepare again to retry the same identity.

Readiness states are actionable:

- `ready`: tasks for the exact revision and lock may be submitted.
- `setup_required`: GitHub installation access is missing or revoked; repair the
  installation or enrollment and retry preparation.
- `failed`: inspect `repository-preparation-*` Job logs, correct checkout,
  lock, capacity, or Ayni failures, then retry.

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev get jobs
kubectl --context docker-desktop -n ayni-orchestrator-dev logs \
  job/REPOSITORY_PREPARATION_JOB -c preparer
```

## Task operation

The browser exposes submission, live phase refresh, cancellation, retry,
requested-input follow-up, draft PR links, and trusted evidence. A task freezes
its repository/profile snapshots, start revision, request, criteria, protected
paths, test-edit authorization, lock, and preparation identity. Retry creates a
linked new task with the same constraints; it never mutates prior evidence.
Requested input is appended only to that linked follow-up.

Cancellation sets durable intent before deleting an active Job. A late worker
cannot revive the task. If the control plane restarts, deterministic Job names,
durable phases, GitHub branch lookup, and immutable evidence allow
reconciliation to continue without a second external effect.

Dependency evolution is explicitly unsupported by the current profile. Candidate edits
to `.ayni.lock`, `Cargo.toml`, `Cargo.lock`, or `rust-toolchain.toml` produce
`dependency_change_unsupported` and no PR. `.ayni.toml` edits produce
`policy_rejected`. A candidate that changes a task-protected path or edits tests
without frozen authorization produces `scope_rejected`; the task UI lists each
violation, and no publication Job is scheduled. Create a separately reviewed
environment update outside the orchestrator, enroll its new lock digest, and
prepare its revision before submitting feature work.

## Update without losing data

Never run cleanup to perform an update. Record PVC identities and task state,
then deploy in place:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev get pvc
curl --fail http://127.0.0.1:18080/v1/development-tasks > /tmp/tasks-before.json
scripts/local/deploy.sh
scripts/local/check.sh
curl --fail http://127.0.0.1:18080/v1/development-tasks > /tmp/tasks-after.json
```

SQLite migrations are additive. Existing tasks, checkpoints, evidence, and
prepared environments remain on their existing claims. Compare known
task IDs before and after. Do not roll back a migrated database to older
binaries.

## Retention, backup, and recovery

Normal cleanup retains all PVCs and credentials and does not request deletion of
task Jobs. Terminal Jobs remain subject to their configured one-hour Kubernetes
TTL; retained state, checkpoints, evidence, and preparation archives live on the
PVCs and have no automatic expiry. Docker reports host images, BuildKit
cache, and local volumes separately; the Kubernetes node also keeps its own
containerd image copy. Repeated unscoped builds therefore used to accumulate
both builder layers and duplicate image tags. The deployment workflow bounds
the build layers with cache mounts and cleans only superseded project tags after a successful rollout.
Watch usage with:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev get pvc
kubectl --context docker-desktop -n ayni-orchestrator-dev get jobs
docker system df
docker exec desktop-control-plane df -h /var
```

Before cluster reset or destructive pruning, scale the control plane to zero and
back up all four claims using a trusted temporary pod or a Docker Desktop volume
backup. At minimum copy `/var/lib/ayni-orchestrator` from
`control-plane-state`; include `development-checkpoints`,
`managed-check-evidence`, and `prepared-environments` to retain full recovery
and audit data. Restore claims before starting the deployment. Check file
ownership remains UID/GID 10001, then run `scripts/local/check.sh` and verify
known task/evidence IDs.

A database-only restore preserves task history but may leave an in-progress task
without its checkpoint or evidence. Such a task must fail closed and be retried
as a linked task; never synthesize success or publish unverified output.

## Cleanup

First cancel active tasks in the UI and wait for cancellation to be visible.
Then:

```bash
scripts/local/cleanup.sh
```

This removes only stateless control-plane resources. It deliberately retains
the namespace, secrets, active Jobs, and all PVCs; terminal Jobs may later be
removed by their configured one-hour TTL. Destructive removal is a separate,
explicit operator action after backup:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev delete pvc \
  control-plane-state development-checkpoints managed-check-evidence prepared-environments
```

That command permanently removes task state and retained evidence. It is not
part of normal cleanup.

## Failure triage

- `pilot_ready: false`: deploy the current profile and configure all GitHub/model
  publication variables; inspect control-plane logs.
- `setup_required`: repair GitHub App installation access; credentials are not a
  repository change.
- preparation `failed`: inspect checkout/preparer/engine containers and desktop
  capacity. An older preparation is intentionally not substituted.
- coding `failed` with repository access diagnostic: restore installation
  access, then retry as a new linked task.
- `waiting_for_input`: use Provide input; the original task remains retained.
- validation failure: inspect the evidence view and Job logs. Only a verified
  `success` manifest can enter publication.
- uncertain PR publication: do not submit another task. The controller searches
  the deterministic head branch and recovers the existing PR before retrying a
  create call.
