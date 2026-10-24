# Local development

The supported stack builds on M1 Slice 3 and runs one control plane, PostgreSQL 17.6, an isolated authenticated NATS/JetStream server, and one retained scripted supervisor on Docker Desktop Kubernetes. The supervisor
provisions isolated conversation workers in a dedicated session namespace. PostgreSQL is the workflow authority; JetStream is durable transport, and broker storage never implies executor ownership.

## Prerequisites

- Docker Desktop Kubernetes with current context `docker-desktop`;
- Helm 4.1.1;
- `kubectl`, Docker, OpenSSL, Rust 1.94, Node 24.14, pnpm 10.31, and Buf 1.71;
- at least 8 GiB free in the Docker Desktop VM before building the two Rust workload images.

The capacity guard does not prune caches, volumes, or unrelated data. Reclaiming Docker storage is an explicit operator decision.

## Deploy and check

```sh
dev/ctl deploy
dev/ctl check --expect-empty
dev/ctl scenario
```

The first deploy creates the namespace and seven immutable Secrets without printing their values:

- `workflow-postgres-admin`, mounted only into PostgreSQL;
- `workflow-postgres-migrator`, mounted only into the migration init container;
- `workflow-postgres-runtime`, mounted only into the serving container;
- `workflow-jetstream-admin`, mounted only into NATS and the stream-provisioning hook;
- `workflow-jetstream-application`, mounted into NATS and the control plane;
- `workflow-jetstream-executor`, mounted into NATS and the executor; and
- `workflow-jetstream-acceptance`, mounted only into NATS for the later acceptance harness.

The executor Secret also owns its fixed local instance ID and Ed25519 seed. The private seed never leaves that Pod or enters broker events. Anonymous access is disabled. Workload ACLs allow only the contracted command, report, targeted-decision, consumer-info/fetch, and ACK subjects; stream administration remains exclusive to the administrative identity.

The pinned broker provisions three file-backed, `discard=new` streams:

| Stream | Subject | Limit |
| --- | --- | --- |
| `ORCHESTRATOR_WORKFLOW_COMMAND_V1` | `orchestrator.workflow.command.conversation.v1` | 25,000 / 256 MiB |
| `ORCHESTRATOR_WORKFLOW_REPORT_V1` | `orchestrator.workflow.report.conversation.v1` | 50,000 / 192 MiB |
| `ORCHESTRATOR_WORKFLOW_DECISION_V1` | `orchestrator.workflow.accepted.conversation.v1.*` | 50,000 / 64 MiB |

The broker launcher encodes Secret-backed passwords as quoted NATS strings in
process memory. This preserves numeric-looking and escaped values without changing
the credentials or writing them to configuration files.

All use seven-day age limits, a two-minute duplicate window, one local replica, and a 260-KiB maximum stored message. The combined 512-MiB stream budget sits below the 1-GiB server budget on a retained 2-GiB PVC. The executor has a retained 512-MiB PVC and enforces a 192-MiB authority ledger plus a 64-MiB quarantine subquota within a 256-MiB combined logical cap.

`dev/ctl check` verifies workload readiness, migration history through ordinal 22, the exact stream limits, retained storage, and executor-ledger initialization. `--expect-empty` additionally refuses a database that already contains a workspace.

## Contributor console

After deploying and checking the stack, run:

```sh
dev/ctl console
```

Open `http://127.0.0.1:3000`. Create or select a workspace, project and conversation,
then send a message. The console shows accepted messages, scripted responses,
activity, waiting states, stable older history, and receipt/cursor diagnostics.
Use Disconnect/Reconnect to exercise observation without cancelling admitted work.
Refresh reloads the selected conversation from authoritative state. Open another
tab to inspect the same conversation or select a different one.

An ambiguous command remains in that tab's pending panel. Retry sends its
original key and exact content; do not clear browser storage to retry it. A
canonical capacity rejection confirms non-admission, clears pending state and
allows a new submission after capacity is available.
The gateway rejects foreign origins and hostnames, so use the printed numeric
loopback URL. Stop with Ctrl-C. After a control-plane Pod restart, rerun the console
command to forward the replacement Pod. `dev/ctl console 3001` selects another port.

To repeat browser acceptance against the running console:

```sh
pnpm exec playwright install chromium
pnpm console:acceptance
```

Run browser acceptance against a deployment separate from an active restart/PVC
inventory acceptance profile. The scenario creates a separate workspace and retains its accepted data. It checks
scripted responses, refresh, two tabs, reconnect, a lost command response followed
by reload/retry, conversation isolation, older history, content escaping and mobile
layout. It also verifies configuration edits, frozen turn inputs, stale-tab
conflicts, separate session identities and replacement retry/wake-up. A browser
fixture verifies that a canonical capacity rejection clears pending state before
legitimate resubmission; the PostgreSQL gate proves actual capacity behavior.
Browser-boundary fixtures additionally check typed cursor expiry and a
bounded slow-reader replay burst; real retention/backpressure is covered by the
cumulative PostgreSQL gate. `CONSOLE_URL=http://127.0.0.1:3001` selects another local console port;
`CONSOLE_EVIDENCE_DIRECTORY=/absolute/path` additionally saves a screenshot.

The console is a trusted local contributor surface with scripted execution by
default. Shared configuration, isolated sessions, opt-in OpenAI calls, reported
usage and cancellation are available. The three read tools use frozen inputs and have no mutation capability. See [console architecture](../architecture/contributor-console.md)
for the transport decision, bounds, security and recovery semantics.

## Plan review

Open **Plans** in a selected conversation. Expand **Write or revise a plan**,
enter objective, scope, constraints and unresolved questions, and optionally add
proposed tasks. Saving retains a new immutable version with the project and
originating conversation. Repository/source references are visibly unverified.

Select a plan to review its content and history. **Edit current version** copies
that version into a local draft. Another tab can publish a newer version, but
watch refresh never overwrites your draft; stale saves require fresh review.
The retry panel preserves ambiguous saves and approvals across reloads.

Resolve scope questions and give every proposed task acceptance criteria before
approval. Review the retained version and digest, check the explicit approval
consent, then choose **Approve reviewed version and admit tasks**. A newer streamed
version never changes what you reviewed; select the plan again and review it
before approval. Approval freezes the plan and atomically admits its complete task
set. The receipt and task detail retain workspace, project, originating conversation,
version and digest. Tasks initially await source verification, runtime
selection and an authorized execution budget. Approval dispatches no coding work;
inspect **Tasks** for the current execution state. A follow-up requires a new plan.

The cumulative browser acceptance includes structured task editing, lost-response
replay, two-tab revision and approval conflicts, exact approval retry, draft preservation, history, task pagination and scope checks.
See [M3 acceptance](plan-acceptance.md) for cumulative outage/restart gates,
[plan review](../architecture/plan-review.md) and [task admission](../architecture/task-admission.md) for limits and retained-state rules.

## Task inspection and controls

Open **Tasks** in a selected conversation to browse admitted work across its plans.
**Refresh tasks** reloads the catalog; **Load more tasks** reads the next bounded
page. Select a task to inspect preparation, coding, managed quality and draft
publication separately. The selected task is restored after reload. Automatic
refresh reads its current state every five seconds while the tab is visible;
uncheck it for manual inspection. A failed read clears stale controls and reports
that current state could not be loaded.

Preparation shows frozen repository, source commit, runtime and execution budgets.
Coding shows the retained outcome and provider state, with expandable accepted
patch and worker identities. Quality shows target completion, exit code and
expandable identities and artifacts. Failed quality preserves coding completion
and blocks publication. Provider uncertainty remains explicit and never enables
a fresh coding retry. Recovery or repair requires an exact reviewed proposal with explicit consent and a bounded
additional allowance, or a new reviewed plan for materially changed inputs.

**Download task selection** supplies the full originating scope and task ID for
the trusted [preparation helper](../architecture/task-execution.md#operator-workflow).
Add `inputs` with `runtimeImage`, `maxSeconds` and `maxTokens` to the downloaded
selection before running verification. Verify source and upload the frozen source
snapshot through the local helper;
the browser cannot read a host checkout or submit source packs. **Cancel task
execution** fences queued or active work. An already sent provider request may
have incurred a charge; cancelling work does not resolve its external outcome.

After reviewing an accepted patch and passing quality evidence, choose **Review
draft publication**. Inspect the exact repository, source, branch, base, title,
body, commit identity and evidence digests. Check the explicit consent and choose
**Authorize draft publication**. This creates publication authority without
rerunning coding or quality. A configured publisher with its own scoped
credential is still required to perform external work.

**Cancel draft publication** is available before draft creation begins. A pushed
branch may remain. Creating or uncertain publication permits reconciliation of the
original request, with no second create command. The retained PR link validates
against the recorded GitHub repository and PR number. External PR state is the
last retained observation, not a live GitHub poll.

Task commands use the same pending panel as plan approval. After a lost response,
reload and **Retry the same command** to replay its exact scope, attempt, evidence
and key. Switching conversations clears review consent; late responses cannot
replace another task's details. Concurrent operators can see stale controls, but
the canonical command rechecks publication eligibility and cancellation fences.

## Task feedback and attention

The originating conversation shows retained execution and quality problems,
including interrupted work, uncertain provider outcomes and failed checks.
Project attention exposes the same notices across its conversations. Open a notice
to inspect its originating task and saved patch or quality evidence.

Feedback is factual system activity, separate from assistant messages. Notices
remain in conversation history after recovery is authorized. Project attention hides those
resolved decisions; the original notice links to the continued attempt and its separate coding
and quality outcomes. Reading them grants no permission to retry or continue. See [task feedback](../architecture/task-feedback.md).

For M6 acceptance, use owned test namespaces on explicit `docker-desktop`.
Preserve stores within restart tests, then purge the owned test deployment and
its ownership-labelled session namespace after testing. Leave unrelated
development environments and retained user work intact.

## Shared configuration and session controls

Open **Shared agent configuration** after selecting a workspace. Save workspace
instructions, provider/model, token limits and idle timeout. These settings apply
to subsequently admitted messages in every conversation in that workspace.
Already admitted turns retain their original configuration revision, including
queued turns. Configuration edits from a stale tab are rejected; refresh the
resources and review the current instructions before saving again.

`scripted`/`scripted` is the default provider/model. Selecting OpenAI requires an
explicit model. Newly admitted turns retain that provider binding; deployment
enablement and a workspace-scoped credential mounted directly into the worker
are required before a paid call. Existing admitted scripted turns remain scripted. The fixed interface version is
v1; displayed configuration revision IDs identify immutable content.

Open **Session** in a conversation's activity panel and select **Refresh session**.
The first eligible message creates a scoped session. Different conversations use
separate retained ledgers and workload generations. After work and report
handoffs settle, **Replace idle session** fences that generation. The next
eligible message starts the next generation. An active session cannot be replaced
through this control; restore its retained executor and ledger instead.

The replacement button uses the server's advisory `can_replace` readiness flag.
While the panel shows finishing accepted work, select **Refresh session** after
the remaining outbox handoffs settle. The replacement command rechecks readiness
transactionally, so a stale enabled button cannot bypass active-work protection.

Context manifests retain at most 10,000 message references. Admission reserves
space for possible predecessor responses before accepting a new input. A capacity
rejection requires available room, an explicit new conversation or reviewed
context policy; the runtime does not silently discard older inputs.

Idle expiry preserves retained evidence. Restart within a valid generation
adopts its retained identity and storage. Work after expiry or replacement starts
the next generation with separate storage. Replacing a generation preserves accepted
conversation history; it does not authorize deleting ledger volumes or retrying
uncertain provider calls. See [session authority](../architecture/conversation-execution.md#isolated-session-authority)
for the full lifecycle contract.

The supervisor provisions workers in `<workflow-namespace>-sessions`, for example
`ayni-orchestrator-dev-sessions`. The parent namespace must be at most 54
characters. Each generation has a distinct 512 MiB result-ledger PVC and immutable
bearer Secret. A worker mounts neither broker/database credentials nor a Kubernetes
API token. Its request listener checks the bearer and complete parent scope.
NetworkPolicy declares supervisor-only ingress and an explicit provider egress policy;
enforcement depends on the local cluster network plugin.

## Provider conversations and recovery

Select **OpenAI** and an explicit model in shared configuration only after the
operator enables OpenAI execution and configures a workspace-scoped credential
for direct session injection. Never paste credentials into instructions, messages
or the browser configuration form. Provider credentials do not pass through the
console, canonical API, broker or transcript.

The **Provider calls and usage** panel shows retained call IDs, the frozen
provider/model, reservations and nullable reported usage. **Unknown** means the
provider did not report that counter; it does not mean zero usage. Select
**Refresh provider calls** to reread the retained projection.

Use **Cancel turn** to cancel a specific queued or active turn. Disconnecting a
watch only stops observation. An ambiguous cancellation command retains its exact
turn, scope and command key; **Retry the same command** replays it. A sending or
uncertain call may have incurred cost even after cancellation is requested.

An uncertain external outcome blocks automatic resending and session retirement.
Restore the retained worker ledger and reconcile the exact call ID. Do not delete
its PVC, replace its identity or generate a new attempt to make the warning go
away. Use the explicit possible-charge acknowledgement only after reviewing the exact
call ID and request digest. It closes the held turn and retires its generation
after the accepted uncertainty report and outbox handoffs settle. Queued work can
then start in a fresh session; the uncertain request is never resent. Unknown
usage, its reservation, the acknowledgement receipt and the old ledger remain
retained. Accepted results replay from retained evidence and appear once.

The deterministic [provider fixture](../../../dev/fixtures/provider/README.md)
exercises request mapping, reported and missing usage, uncertainty, cancellation
receipts and reconnect without a real provider credential. Its browser acceptance
is separate from the scripted console suite and does not establish real-provider
latency or availability. The final M2 fixture also exercises scoped tool reads and tool-limit failures.
The initial tool profile requires a model that emits function calls without
reasoning items; mixed reasoning/tool responses fail explicitly as
`provider_tool_rejected`.

## Happy-path diagnostic

`dev/ctl scenario` executes the credential-free diagnostic inside the control-plane Pod. It uses the pod-loopback private-v1 listener to create a workspace, project, and conversation; admit one message; observe the signed executor claim and accepted scripted response through a watch, reconnect after each update; and verify the terminal snapshot and stable history. It prints generated resource IDs and the final revision, never credentials or message content.

For manual access, port-forward the Pod rather than the Service:

```sh
pod=$(kubectl --context docker-desktop -n ayni-orchestrator-dev get pod \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
  -o jsonpath='{.items[0].metadata.name}')
kubectl --context docker-desktop -n ayni-orchestrator-dev \
  port-forward "pod/$pod" 50051:50051

cargo run -p ayni-orchestrator-control-plane --bin workflow-diagnostic
```

Health and readiness remain separate:

```sh
kubectl --context docker-desktop -n ayni-orchestrator-dev \
  port-forward service/control-plane 8080:8080
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
curl --fail http://127.0.0.1:8080/metrics
```

Broker unavailability leaves PostgreSQL readiness honest and delivery pending. PostgreSQL unavailability makes the command/snapshot boundary unavailable.

## Watch recovery and retention

Keep the last received cursor to resume `WatchConversation`. An empty cursor starts with a consistent snapshot; a valid cursor starts with the next durable update. Heartbeats have no cursor. A disconnected watch never cancels admitted work.

`WATCH_CURSOR_EXPIRED` means the cursor predates the seven-day watermark. Reconnect without a cursor to recover authoritative current state. `RESOURCE_EXHAUSTED` closes a slow reader; resume from its last consumed cursor. Cleanup bounds each pass and preserves immutable receipts and unresolved transport evidence.

`/readyz` names a full application report quarantine or unsupported retained version. Stop the incompatible producer, deploy a compatible consumer, and allow retained replay to resolve the entries. Malformed entries retain identity/digest diagnostics for eight days; unsupported exact bytes have no automatic expiry.

## Restart and retention check

After a successful scenario, restart each owner and rerun the checks:

```sh
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart statefulset/workflow-postgres
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart statefulset/workflow-jetstream
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart deployment/control-plane
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout restart statefulset/scripted-executor

dev/ctl check
dev/ctl scenario
```

PostgreSQL, JetStream, and executor-ledger PVCs survive ordinary restarts and Helm uninstall. The control plane reruns schema/checksum/privilege validation, exact outbox bytes remain authoritative until claim acceptance, and the executor resumes retained reports before accepting new work.

## Validation gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
scripts/ci/check-workflow-postgres-feasibility.sh
scripts/ci/check-workflow-private-v1-feasibility.sh
scripts/ci/check-helm.sh
pnpm proto:check
pnpm docs:check
```

The PostgreSQL gate covers migrations through ordinal 22, frozen configuration, scoped context and session generations, signed claim/terminal acceptance, duplicate report decisions, accepted response insertion, and conversation-only FIFO context promotion in addition to the Slice 1 authority checks. Task-quality coverage additionally verifies immutable evidence retries, accepted snapshot/runtime identities, scoped reads and cancellation/deadline fencing. Task feedback coverage verifies failed quality, interrupted work, empty success and exact notice replay through owner restarts. The Helm gate validates pinned broker images, exact stream budgets, role ACL shape, Secret separation, and retained PVCs. `dev/ctl scenario` is the deployable Docker Desktop evidence.

After browser acceptance creates at least two conversations, inspect the deployed
session boundary without reading Secret data:

```sh
python3 scripts/ci/check-session-topology.py --context docker-desktop --namespace ayni-orchestrator-dev
```

The check verifies retained namespace ownership, full worker scope, distinct
per-generation PVCs, token mount boundaries and supervisor RBAC. It does not
establish NetworkPolicy enforcement by the cluster network plugin.

## Managed task quality acceptance

Build and load the dependency-free [managed fixture](../../../dev/fixtures/task-quality/README.md)
using the pinned Ayni 0.16.2 CLI:

```sh
dev/ctl quality-runtime
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-quality \
  AYNI_ORCHESTRATOR_TASK_FIXTURE=true dev/ctl deploy
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-quality \
  dev/ctl acceptance /absolute/new/evidence-directory --quality-only
```

The quality profile also runs real browser recovery acceptance; install Chromium once with
`pnpm exec playwright install chromium`. It prepares a separate failed-quality task whose
recovery decision is left to the browser, then exercises two-tab answers and consent,
lost-response replay across reload and conversation navigation, and continued outcomes.

The runtime command records its local manifest digest in
`.ayni/task-quality-runtime.json`; acceptance freezes that exact image. Set
`AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE` to the printed local digest to select it
explicitly. Nothing is published to a registry. The selected context remains
`docker-desktop` and the acceptance namespace must use the shown prefix.

This profile includes the existing coding cancellation, Pod loss, supervisor
adoption and retained-volume restart gates. It also runs actual Ayni dependency
preparation and checks in the selected runtime, retains passing and failing
quality evidence separately from successful scripted coding, and rereads each
result after control-plane, supervisor, executor, broker and PostgreSQL restarts.
The fixture has no external dependencies. Its unsigned certificate proves local
lock/runtime consistency and protected content; it does not claim a trusted
builder signature or real paid-provider execution. See the
[quality contract](../architecture/task-quality.md) for evidence requirements and
failure boundaries. Retained acceptance state is preserved for inspection.

## Task publication acceptance

Use the actual managed Ayni 0.16.2 fixture runtime and an isolated namespace:

```sh
dev/ctl quality-runtime
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-publication \
  AYNI_ORCHESTRATOR_TASK_FIXTURE=true dev/ctl deploy
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-publication \
  dev/ctl acceptance /absolute/new/publication-evidence --publication-only
```

Install the browser once with `pnpm exec playwright install chromium`.
The profile first runs the cumulative quality gates, then builds and loads the
local publication fixture. It binds the publisher's workspace and repository
explicitly in Helm after creating the approved source task. It supplies only a
public synthetic fixture token, never an existing GitHub credential.

The fixture uses a real bare Git remote and GitHub-compatible HTTP responses.
Its Git objects and PR ledger have retained storage. The profile loses a create
response after durable acceptance, verifies one exact draft/commit, rejects failed
quality, cancels an unclaimed intent and checks branch-collision fencing. It then
restarts every owner, including the fixture, and verifies the original identity.
A closed external PR is retried through the same retained intent without creating
another draft. It also exercises canonical publication preview, consent, a lost
accepted response, reload/exact retry and queued cancellation through the browser
on a separate managed-quality task. Desktop/mobile screenshots and retained
command/evidence identities are saved under `console/`.

This establishes actual Git transport and protocol recovery. It does not establish
live GitHub authentication or compatibility with protected-branch signing policy.
The [publication contract](../architecture/task-publication.md) describes the
separate credential boundary and trusted operator submission workflow.

## Complete task delivery acceptance

Use the [M4 acceptance matrix](task-acceptance.md) and the cumulative profile:

```sh
export AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-m4
dev/ctl quality-runtime
AYNI_ORCHESTRATOR_TASK_FIXTURE=true dev/ctl deploy
dev/ctl acceptance /absolute/new/m4-evidence --m4
```

This extends the publication profile with a live stale-worker cancellation fence,
publisher interruption around draft creation, uncertainty without a remote PR and
retained cleanup/redeployment. It preserves all authoritative stores and writes
`acceptance.json` only after the complete deployed matrix passes. Use a separate
deployment for ordinary console acceptance to keep retained PVC inventories stable.
The matrix documents fixture boundaries, required repository checks and recovery.

## Cleanup

```sh
dev/ctl cleanup
```

Cleanup stops the Helm release and supervisor, then removes owned session Pods.
It preserves PostgreSQL, JetStream, supervisor/session ledger PVCs, immutable
Secrets and the owned session namespace. A later deploy reuses them. Do not roll back while a command/report/decision handoff is unresolved; repair forward or intentionally reset the disposable namespace.

Permanent deletion requires the exact local namespace:

```sh
dev/ctl purge ayni-orchestrator-dev
```

The purge command refuses absent, unlabelled, or non-local namespaces. It deletes the exact parent namespace and its ownership-labelled session
namespace, including all Secrets and retained workflow/session stores.
