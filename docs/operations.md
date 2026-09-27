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

The core check requires `pilot_ready: true`, fetches the HTML marker and
repository and task list APIs, verifies the application metrics endpoint, and
does not execute browser JavaScript. The first install creates four retained
PVCs. Re-running the install does not replace retained claims. The optional
observability stack is installed and checked separately as described below and
uses only disposable `emptyDir` storage.

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

Use the UI to save repository enrollment. No model-specific agent profile is
configured. The control plane reads the existing `openai-api` Secret from its
read-only projected volume, asks OpenAI for the authenticated key's model list
through Rig, filters it to the currently supported Responses/tool-capable model
families, and caches successful discovery for five minutes. The UI submits
the selected provider model ID, while the task freezes an internal provider,
model, and Secret-reference snapshot for execution and retry. `GET
/v1/provider-models` exposes only sanitized provider model metadata, never the
credential. The current compatibility policy admits GPT-5, GPT-4.1, GPT-4o,
o1, o3, and o4 families while excluding audio, realtime, transcription, TTS,
image, search, and instruct variants. Existing retained tasks and legacy profile
rows remain readable but are not shown as model choices.

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
its repository and selected-provider-model snapshots, start revision, request,
criteria, protected paths, test-edit authorization, lock, and preparation identity. Retry creates a
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

## Pilot model-usage measurement

Both model-consuming stages (`coding` and `pr_preparation`) write one bounded,
prompt-free report for every provider interaction. Reports are first atomically
spooled in a task-scoped, mode-0700 subpath of the retained development-checkpoint
claim that repository commands cannot access, and are reconciled into SQLite with
the unique identity `(task_id, stage, call_id)`. Coding file, shell, and Git tools
run under a distinct UID with an allowlisted environment and no capabilities. The
trusted agent transport retains only the narrow filesystem and identity-switching
capabilities needed to protect the spool, and fails closed if the tool identity can
read or write the usage mount. Re-reading a report,
retrying reconciliation, or restarting the control plane does not create a second
interaction. A worker stopped during a request leaves an in-progress spool record;
once the Job is terminal it becomes a failed or cancelled interaction with unknown
usage instead of an asserted zero.

The development-task list and detail APIs return `model_usage.interactions`,
`model_usage.by_stage`, and `model_usage.total`. `collection_status` is
`collecting` while a model stage can still add calls, `complete` only after its
last model Job is terminal, and `unavailable` for tasks created before this
instrumentation. The task UI renders the same values. Use these definitions for
pilot measurements:

- **Input, output, cached-input, reasoning, and total tokens** are raw counters
  supplied by the provider for each HTTP model interaction. Cached-input and
  reasoning tokens are subsets of the provider's input and output counts; do not
  add them again to total tokens. No count is inferred from prompt text.
- **Stage token total** is the sum of a field only when every interaction in that
  stage reported that field. **Task token total** applies the same rule across
  both stages. A missing field is `null`/`unknown`, never zero. A stage with no
  interactions is also unknown rather than zero.
- **Usage unavailable** counts interactions for which the provider returned none
  of the supported counters. Partial provider usage retains the reported fields
  and leaves the others unknown, including on failed work when the provider
  supplied counters.
- **Estimated cost** is not an invoice or provider billing truth. It is calculated
  only for a known pricing entry and stores the currency, exact priced token
  inputs, per-million-token rates, and pricing-table identity with the interaction.
  The requested task-profile model and nullable provider-resolved model identity
  are retained separately. The current table identity is
  `openai-public-2025-08-07`, in USD micro-units, and applies only when the
  provider reports the exact `gpt-5-2025-08-07` identity; floating aliases and
  missing resolved identities remain unpriced. Its fixed rates are USD 1.25 per
  million input tokens, USD 0.125 per million cached-input tokens, and USD 10.00
  per million output tokens. Cached input is priced at the cached rate when
  reported; when the optional
  cached breakdown is absent, all reported input is conservatively priced at the
  standard input rate. OpenAI reasoning tokens are already included in output and
  are not charged a second time. A stage or task estimated cost is unknown when
  any included interaction cannot be priced.
- **Latency** is wall time from issuing one model request through its response or
  terminal error. Outcomes are `succeeded`, `failed`, or `cancelled` for each
  interaction and do not imply the enclosing task outcome.

`GET /metrics` exports aggregate task phases/outcomes/durations, stage
outcomes/durations, model interactions/latency/tokens, provider failures, and
reconciliation failures. Durable metrics are reconstructed from SQLite;
reconciliation counters are process-local and correctly reset when the control
plane restarts. Labels are limited to phase, stage, outcome, provider, selected
model, token kind, and reconciler. The endpoint intentionally omits task IDs,
request text, repositories, model-generated content, reason strings, and
arbitrary errors. Usage records and metrics never retain prompts, completions,
reasoning text, tool transcripts, credentials, authorization headers, or
provider response bodies.

## Optional local task and resource dashboard

The core deployment does not require monitoring. Install the version-controlled
observability deployment only when an operator needs the dashboard:

```bash
scripts/local/deploy-observability.sh
scripts/local/check-observability.sh
```

The stack uses pinned, off-the-shelf Prometheus, kube-state-metrics, and Grafana
images. Prometheus scrapes every 15 seconds and keeps no more than 48 hours or
900 MB in a 1 GiB `emptyDir`. That window preserves resource samples after a
terminal Job's one-hour TTL removes its Pod. A Prometheus restart or teardown
may still discard all history. SQLite task lifecycle and exact provider-usage
records remain authoritative and do not depend on monitoring.

The provisioned **Ayni Orchestrator · Tasks and resources** dashboard includes:

- active task phases, terminal outcomes, stage outcomes, and stage/total p95
  duration from durable application telemetry;
- aggregate provider token usage by stage and selected model, provider latency
  and errors, and reconciliation errors;
- CPU and memory by stage and by container, including the validation `engine`
  restartable init container, plus usage versus declared limits;
- restarts, OOM/error/deadline termination signals; and
- retained-storage pressure and growth over the selected time range, with
  per-PVC volume stats when Docker Desktop exports them and a node-disk fallback.

Application metrics use bounded stage, phase, outcome, provider, model, token
kind, and reconciler labels. They never contain task IDs, repository IDs,
request text, credentials, prompts, completions, reason strings, or task
content. Exact per-task history remains in the API/UI. kube-state-metrics
allowlists only the existing orchestrator ownership, task-ID, and stage labels
needed to join short-lived Kubernetes resource series. The dashboard's task
variable contains only that opaque task identifier; it does not contain task
content. Kubernetes series disappear under the Prometheus retention bounds.

Open Grafana only through a loopback port-forward:

```bash
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward service/grafana 13000:3000
open http://127.0.0.1:13000/d/ayni-orchestrator-tasks-resources
```

Anonymous access is enabled with Viewer permissions because this is the trusted
local profile. Grafana and Prometheus Services remain `ClusterIP`; never expose
them through an Ingress, `NodePort`, or non-loopback forward. For direct query
inspection, use a separate loopback forward to `service/prometheus` on
`19090:9090`. No dashboard variable or metric label contains a credential.

The stack requests 100 millicores and 240 MiB in total. Its aggregate limits are
650 millicores and 768 MiB, plus bounded ephemeral storage (1 GiB Prometheus,
256 MiB Grafana data, and 64 MiB Grafana scratch). These limits leave the
4 CPU/9 GiB validation engine and its worker within the documented 6 CPU/14 GiB
Docker Desktop profile. Monitoring failure cannot block task orchestration.

Remove only the optional resources, RBAC, and disposable history with:

```bash
scripts/local/teardown-observability.sh
```

This preserves the namespace, product Deployments, Jobs, Secrets, and all four
retained PVCs. `scripts/local/cleanup.sh` also tears down observability before
removing the stateless control plane.

Troubleshooting:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev get pods \
  -l app.kubernetes.io/part-of=ayni-orchestrator-observability
kubectl --context docker-desktop -n ayni-orchestrator-dev logs deployment/prometheus
kubectl --context docker-desktop -n ayni-orchestrator-dev logs deployment/kube-state-metrics
kubectl --context docker-desktop -n ayni-orchestrator-dev logs deployment/grafana
```

If an `up` target is missing, rerun `check-observability.sh` and inspect the
matching Deployment logs. A `403` from a kubelet/cAdvisor target indicates that
the metric-only `nodes/metrics` ClusterRole or binding is absent; redeploy the
optional stack rather than granting `nodes/proxy` or broader access. Empty task resource panels
mean no matching Pod was sampled during the selected 48-hour window, or the
selected task predates the current ownership/stage labels. When Docker Desktop
does not export per-PVC kubelet volume stats, the storage panels deliberately
fall back to Docker Desktop root-disk usage/growth and show PVC provisioned
capacity separately; product PVC state is unaffected. Shared-cluster or
production use requires separate review for
authentication, TLS, network isolation, durable monitoring storage, and RBAC.

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

This removes the optional observability stack and stateless control-plane
resources. It deliberately retains the namespace, secrets, active Jobs, and all
PVCs; terminal Jobs may later be removed by their configured one-hour TTL. Destructive removal is a separate,
explicit operator action after backup:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev delete pvc \
  control-plane-state development-checkpoints managed-check-evidence prepared-environments
```

That command permanently removes task state and retained evidence. It is not
part of normal cleanup.

## Lifecycle and structured-log troubleshooting

Every development-task phase change is written transactionally to the append-only
`lifecycle` array returned by both development-task API endpoints. Events contain
an event time, logical stage (`queue`, `coding`, `validation`, or
`pr_preparation`), outcome, and a stable `reason_code` for blocked or terminal
outcomes. Stage completion and the next queued stage share a timestamp, so
operators can calculate queue, coding, validation, PR-preparation, and total
elapsed time without the Kubernetes Job. `lifecycle_complete: false` means the
task predates lifecycle instrumentation; its single migration snapshot is not a
complete duration record. Retry and requested-input actions create a new task
with its own timeline and retain `retry_of` on the new task.

The control plane and every worker binary emit newline-delimited JSON to stdout.
The shared envelope contains `timestamp`, `level`, `target`, `message`, and span
context. Depending on the operation, bounded correlation fields include
`request_id`, `task_id`, `attempt_id`, `preparation_id`, `stage`, `job_name`,
`provider`, `model`, `repository_id`, `status`, `latency_ms`, and `reason_code`.
HTTP responses return `x-request-id`; a caller-supplied request ID is accepted
only when it is a 1-64 character ASCII token. Set `RUST_LOG` to one of `error`,
`warn`, `info`, `debug`, or `trace` to change filtering for orchestrator-owned
targets. Third-party targets remain disabled even at `trace` so dependency
instrumentation cannot expose credential-bearing provider requests.

Logs deliberately exclude authorization headers, credentials, secret values,
prompts, acceptance criteria, repository contents, command output, and
environment dumps. High-cardinality task and repository identifiers and detailed
safe diagnostics belong in logs or durable task records, never metric labels.
Worker child-command streams are captured, retained as evidence where supported,
or suppressed rather than mixed into the JSON stream. The shared redactor is a
last line of defense; do not add sensitive values as tracing fields.

Useful local filters are:

```bash
# Control-plane failures for one task.
kubectl --context docker-desktop -n ayni-orchestrator-dev logs \
  deployment/control-plane | jq -c \
  'select(.task_id == "TASK_ID" or .span.task_id == "TASK_ID")'

# Follow one deterministic Job and filter warnings/errors.
kubectl --context docker-desktop -n ayni-orchestrator-dev logs -f \
  job/JOB_NAME | jq -c 'select(.level == "WARN" or .level == "ERROR")'

# Correlate an HTTP response with its request log.
kubectl --context docker-desktop -n ayni-orchestrator-dev logs \
  deployment/control-plane | jq -c \
  'select(.request_id == "REQUEST_ID" or .span.request_id == "REQUEST_ID")'
```

Kubernetes retains terminal Jobs for one hour in this profile, so pod logs may
expire with them and Docker Desktop log retention is not an audit store. The
SQLite lifecycle remains until the `control-plane-state` claim is explicitly
deleted. There is no centralized log backend, tracing exporter, automatic log
backup, or guaranteed log retention; collect relevant JSON logs before Job,
namespace, or Docker Desktop cleanup.

## Failure triage

- `pilot_ready: false`: deploy the current profile and configure the GitHub
  publication variables and `openai-api` model-catalog Secret; the readiness value
  `model_catalog: "unavailable"` means discovery has not yet succeeded, so inspect key validity,
  provider reachability, and control-plane logs.
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
