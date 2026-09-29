# Local operations

This runbook is the operating contract for the supported Docker Desktop
profile: one trusted maintainer team, one local cluster, and the allowlisted
`ayni-oss/sandbox` repository. It is not a multi-tenant service contract.

## Access boundary

The browser server and control plane have no end-user authentication in this
profile. Both Services are `ClusterIP`; do not expose them with an Ingress,
`NodePort`, or a non-loopback forward. Open the SvelteKit server boundary only
through:

```bash
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward --address 127.0.0.1 service/web 18080:8080
open http://127.0.0.1:18080/
```

The Node server runs as a sidecar in the control-plane Pod and calls
`127.0.0.1:50051`. It receives no Kubernetes service-account token or credential
volume. The control-plane HTTP listener is operational only: `/livez`, `/readyz`,
and `/metrics`, plus the bearer-protected internal publication callback. It does
not serve the retired embedded UI or product JSON routes.

Native gRPC uses a separate pod-loopback listener. The Service declares the
target for `kubectl port-forward`, but normal Service traffic addresses the pod
IP and cannot reach the listener:

```bash
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward --address 127.0.0.1 service/control-plane 15051:50051
grpcurl -plaintext -max-time 5 127.0.0.1:15051 list
```

Never change `ORCHESTRATOR_GRPC_BIND_ADDRESS` to a non-loopback address in this
unauthenticated profile. Approved custom clients use the separate opt-in
`customClients.enabled` application-chart value (nested under
`ayni-orchestrator` in the local umbrella) and TLS port 50052; do not repurpose
the loopback listener. Identity provisioning, port-forward discovery, certificate
rotation, revocation, capability review, limits, and audit response are covered
in [Custom-client gRPC access](custom-client-access.md).

The same-origin SvelteKit loads, actions, and bounded
server-sent task updates are private browser adapters, not a second supported
product protocol. Anyone able to reach the browser surface or native gRPC
can operate the product. Kubernetes access,
Docker Desktop host access, the GitHub App installation, and the configured
model key are therefore privileged. The internal publication endpoint also
requires its dedicated bearer token. Neither GitHub nor model credentials are
returned by the API or UI.

## Prerequisites and capacity

- Docker Desktop Kubernetes 1.32 or newer, with a `docker-desktop` context
  available; local scripts select it explicitly and never fall back to the
  current context.
- Helm 4.1.1, `kubectl`, Docker, Git, Rust, Node, pnpm, `curl`, and `grpcurl`.
- At least 6 CPUs, 14 GiB memory, and 30 GiB free disk assigned to Docker
  Desktop. One managed validation/preparation engine requests 4 CPUs and 9 GiB.
- A GitHub App installed on the enrolled repository with Contents and Pull
  requests read/write permissions.
- A committed `.ayni.toml` and `.ayni.lock`; the worker uses Ayni `0.13.0`.
- The local credential file described in `dev/.env.example`. Never commit it.

Storage claims are 1 GiB for SQLite state, 2 GiB each for checkpoints and
validation evidence, and 10 GiB for prepared image archives. The deploy script
requires at least 12 GiB free before an image build/import, then removes only
obsolete `local/ayni-orchestrator-*` image tags not referenced by a pod. Rust
Cargo registry and target data use named BuildKit cache mounts so source changes
do not create a new multi-gigabyte immutable builder layer each time. This does
not prune shared cache or Docker volumes. Docker Desktop's
single node permits the selected `ReadWriteOnce` sharing pattern. Claim sizes in
the chart are install-time minimums: do not reduce them, and expand them only
when the active storage class supports expansion. Do not copy this topology to
a multi-node cluster.

Current profile limits are one repository allowlist, 24 coding-agent turns, 4 PR
summary turns, one-hour preparation/validation Job deadlines, 5 GiB checkout
workspaces, 20 GiB isolated engine storage, and no automatic deletion of
retained state. Kubernetes capacity naturally serializes the 4 CPU/9 GiB heavy
Jobs on the recommended desktop allocation. Maintainers should submit one
change at a time.

## Clean installation

From a clean Docker Desktop Kubernetes namespace:

```bash
dev/ctl init
# Fill dev/.env locally; do not print it.
dev/ctl secrets
dev/ctl deploy
dev/ctl check
```

The core check requires `pilot_ready: true`, verifies the operational and
metrics endpoints, fetches the server-rendered SvelteKit marker backed by real
gRPC repository, model, and task reads, and checks native gRPC health and
reflection. It does not execute browser JavaScript. The first install creates
four retained PVCs. Re-running the install does not replace retained claims.
The same release includes the observability stack described below and uses only
disposable `emptyDir` storage for its monitoring data.

To reproduce a clean installation without disturbing the normal development namespace,
use an isolated namespace and local port, then remove that namespace after
recording the result:

```bash
export AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-clean-check
export AYNI_ORCHESTRATOR_WEB_LOCAL_PORT=18082
export AYNI_ORCHESTRATOR_OPERATIONS_LOCAL_PORT=18083
export AYNI_ORCHESTRATOR_GRPC_LOCAL_PORT=15052
dev/ctl secrets
dev/ctl deploy
dev/ctl check
dev/ctl purge "$AYNI_ORCHESTRATOR_NAMESPACE"
unset AYNI_ORCHESTRATOR_NAMESPACE AYNI_ORCHESTRATOR_WEB_LOCAL_PORT \
  AYNI_ORCHESTRATOR_OPERATIONS_LOCAL_PORT AYNI_ORCHESTRATOR_GRPC_LOCAL_PORT
```

Namespace deletion is intentionally explicit because it deletes that isolated
installation's Secrets, Jobs, and retained claims. Never point this command at
the normal development namespace.

Use the UI to save repository enrollment. No model-specific agent profile is
configured. The control plane reads the existing `openai-api` Secret from its
read-only projected volume, asks OpenAI for the authenticated key's model list
through Rig, filters it to the currently supported Responses/tool-capable model
families, and caches successful discovery for fifteen minutes so paginated
catalog snapshots remain resumable for the documented token lifetime. The UI submits
the selected provider model ID, while the task freezes an internal provider,
model, and Secret-reference snapshot for execution and retry. The server BFF
returns only sanitized provider model metadata from native gRPC, never the
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

## Local task and resource dashboard

The `local` umbrella installs version-locked upstream Prometheus,
kube-state-metrics, and Grafana chart dependencies by default.
`dev/ctl check` validates them together with the reusable application
chart; rerun only `dev/ctl test observability` when troubleshooting
the monitoring path. Prometheus scrapes every 15 seconds and keeps no more than 48 hours or
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
  port-forward --address 127.0.0.1 service/grafana 13000:3000
open http://127.0.0.1:13000/d/ayni-orchestrator-tasks-resources
```

Anonymous access is enabled with Viewer permissions because this is the trusted
local profile. Grafana and Prometheus Services remain `ClusterIP`; never expose
them through an Ingress, `NodePort`, or non-loopback forward. For direct query
inspection, use a separate loopback forward to `service/prometheus` on
`19090:9090`. No dashboard variable or metric label contains a credential.

The stack requests 110 millicores and 256 MiB in total, including the
Prometheus configuration reloader. Its aggregate limits are 700 millicores and
800 MiB, plus bounded ephemeral storage (1 GiB Prometheus, 64 MiB
memory-backed Grafana data, and 64 MiB Grafana scratch). These limits leave the
4 CPU/9 GiB validation engine and its worker within the documented 6 CPU/14 GiB
Docker Desktop profile. Monitoring failure cannot block task orchestration.

The observability resources are part of the local release. Normal
`dev/ctl cleanup` uninstalls them with the stateless application while
preserving the namespace, Jobs, Secrets, and all four retained PVCs.

Troubleshooting:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev get pods \
  -l orchestrator.ayni.dev/stack=observability
kubectl --context docker-desktop -n ayni-orchestrator-dev logs \
  deployment/prometheus --container prometheus-server
kubectl --context docker-desktop -n ayni-orchestrator-dev logs deployment/kube-state-metrics
kubectl --context docker-desktop -n ayni-orchestrator-dev logs deployment/grafana
```

If an `up` target is missing, rerun `dev/ctl test observability` and inspect the
matching Deployment logs. A `403` from a kubelet/cAdvisor target indicates that
the metric-only `nodes/metrics` ClusterRole or binding is absent; redeploy the
local release rather than granting `nodes/proxy` or broader access. Empty task resource panels
mean no matching Pod was sampled during the selected 48-hour window, or the
selected task predates the current ownership/stage labels. When Docker Desktop
does not export per-PVC kubelet volume stats, the storage panels deliberately
fall back to Docker Desktop root-disk usage/growth and show PVC provisioned
capacity separately; product PVC state is unaffected. Shared-cluster or
production use requires separate review for
authentication, TLS, network isolation, durable monitoring storage, and RBAC.

## Update without losing data

Never run cleanup to perform an update. Record PVC identities and task state,
then deploy in place. The first deployment after Decision 0001's manifest flow
uses Helm's explicit `--take-ownership` and guarded server-side field-conflict
migration for the same-named application resources, including the four claims;
it does not delete or recreate retained state. A release created by the first
Decision 0006 chart revision also receives a one-time guarded replacement of
its three disposable monitoring Deployments because the upstream dependency
charts use different immutable selectors. Keep the fixed `ayni-orchestrator`
release name and verify claim UIDs and known task IDs before and after either
one-way migration.

Most legacy objects carry the expected Ayni ownership label. Four objects from
the superseded Kustomize flow do not. Inspect those objects and all colliding
names first, then authorize that migration for one invocation only:

```bash
AYNI_ORCHESTRATOR_ADOPT_UNLABELED_LEGACY_RESOURCES=true \
  dev/ctl deploy
```

Never set this flag for routine upgrades or an unfamiliar namespace. The script
still validates the known legacy signatures, refuses foreign ownership
annotations or management labels, and stops on any inspection error.

Before deployment, record the retained PVC identities and known task IDs through
the browser or native gRPC. Deploy in place, run the smoke checks, and confirm the
same task IDs remain visible:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev get pvc
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward --address 127.0.0.1 service/control-plane 15051:50051
# In another terminal, before and after deployment:
grpcurl -plaintext -max-time 10 -d '{}' 127.0.0.1:15051 \
  ayni.orchestrator.v1.DevelopmentTaskService/ListDevelopmentTasks
dev/ctl deploy
dev/ctl check
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
backup. At minimum copy `/data/orchestrator.sqlite` from the
`control-plane-state` claim mounted at `/data`; include `development-checkpoints`,
`managed-check-evidence`, and `prepared-environments` to retain full recovery
and audit data. Restore claims before starting the deployment. Check file
ownership remains UID/GID 10001, then run `dev/ctl check` and verify
known task/evidence IDs.

A database-only restore preserves task history but may leave an in-progress task
without its checkpoint or evidence. Such a task must fail closed and be retried
as a linked task; never synthesize success or publish unverified output.

## Cleanup

First cancel active tasks in the UI and wait for cancellation to be visible.
Then:

```bash
dev/ctl cleanup
```

This uninstalls the `ayni-orchestrator` Helm release, including the disposable
observability stack and stateless control-plane resources. It deliberately
retains the namespace, secrets, active Jobs, and all PVCs; terminal Jobs may
later be removed by their configured one-hour TTL. Destructive removal is a
separate, explicit operator action after backup:

```bash
kubectl --context docker-desktop -n ayni-orchestrator-dev delete pvc \
  control-plane-state development-checkpoints managed-check-evidence prepared-environments
```

That command permanently removes task state and retained evidence. It is not
part of normal cleanup. After backup, remove the entire labeled local environment
(including its Secrets and Jobs) by typing the exact namespace:

```bash
dev/ctl purge ayni-orchestrator-dev
```

`purge` refuses absent, unlabeled, or non-`ayni-orchestrator-*` namespaces.

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
