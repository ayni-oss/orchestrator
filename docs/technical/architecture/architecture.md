# Architecture

The [product flow](../guides/ayni-product-flow.md) defines shared repository/runtime ownership.
[Assurance](assurance.md) and [environment changes](../guides/environment-changes.md) describe current proposal support and the remaining candidate-check boundary.

The [event platform](event-platform.md) describes implemented authenticated reporting,
transactional acceptance, JetStream delivery and diagnostic tracing.
[Code organization](code-organization.md) defines library ownership and dependency direction.

The control plane admits tasks, freezes repository and runtime identity, model,
constraints, total coding turns, quality repair limit and deadline. It schedules
one worker, supplies repository-scoped credentials, observes durable progress and
results, and handles cancellation and cleanup. It never pushes task Git changes
or opens task pull requests.

The worker attests the runtime before checkout, verifies the source and managed
lock, codes with focused `ayni_verify` feedback, commits coherent milestones,
and always runs `ayni_check` against the full repository. Actionable failures can
start bounded repair sessions within the same total turn budget and deadline.
The final evidence must bind the checked source and final commit. Missing or
malformed evidence is unavailable and cannot pass.

Execution, quality and publication are independent outcomes. Valid changed work
can become a draft with failed or unavailable quality. Admission failures,
unsafe changes, cancellation and no-change outcomes do not publish. The worker
persists publication intent before push and a receipt before completion. It
adopts a matching draft after uncertain provider responses and stops on a
conflicting remote head.

Atomic recovery generations retain changes, local history, phase and consumed
budgets without adding Git commits. Recovery after publication begins resumes
publication. Retry publication uses retained final work without another coding
session; ordinary task retry creates a new attempt.

The API exposes frozen task snapshots and attempt-owned worker progress, results
and pull request receipts. SQLite schema version 25 retains preparation state,
authenticated result inboxes, accepted decisions, transactional outboxes and
operator lifecycle facts. Recognized versions 6–24 migrate in order before
reconciliation; malformed retained state aborts the upgrade. Older and unknown
versions are rejected. See [operations](../operations/operations.md) for upgrade
and rollback requirements. Credentials remain separate inputs and are never worker output.

## Connection credentials

GitHub App private keys and GHCR tokens live in namespace-scoped Kubernetes
Secrets. SQLite stores ownership, safe provider metadata, Secret references and
UIDs, active/candidate revisions and bounded test results. Credential writes are
never stored as generic idempotency request payloads or returned by read APIs.
The connection service is exposed only on the trusted internal listener used by
the operator BFF; the custom-client listener cannot manage credentials.

One configured GitHub App and multiple GHCR connections support two ownership
modes. Explicit deployment references register administrator-managed connections;
these remain read-only in the UI. UI-managed connections use labelled Secret
revisions whose ownership and UID are checked before mutation. Unlabelled Secrets
are never discovered or adopted automatically. Kubernetes RBAC is namespaced;
ownership labels do not restrict Secret access.

Connection reconciliation runs independently of runtime execution. The shared
GitHub credential provider serves both application operations and runtime
reconciliation, polling referenced revisions every 30 seconds. UI writes queue
validation and activation; no control-plane restart is required. A failed
replacement keeps the previous valid revision active and exposes its failure.
Missing or disabled active credentials block new authenticated operations.

GHCR authentication, push, pull and artifact cleanup run in scoped, digest-pinned
probe Jobs. Probe results are persisted before Job cleanup. Push credentials go
only to trusted builders and probes; candidate workloads receive a separate
pull-only Secret. Probe Jobs receive no Kubernetes API token, model or signing
credentials. Each logical runtime profile selects an approved connection. The connection alone
defines its image namespace;
runtime admission freezes the destination and credential revision for subsequent
work and retries. Runtime selections survive source sync and rebuilds. A destination
change creates a new selected revision and removes active admission authority.
Already admitted work retains its frozen destination and credentials; superseded
builds can finish but cannot activate the new selection. Secret retention follows
these frozen references, as described
in [operations](../operations/operations.md).


## Signing certificates

Global Ed25519 certificates are trusted-operator resources. The certificate
service and runtime certificate setter reject custom-client credential management.
Import requests carry write-only seeds, and generation uses operating-system
randomness. Private seeds and key IDs live in owned immutable Kubernetes Secret
revisions; SQLite and browser DTOs hold public keys, references, status and usage.
Explicit administrator Secret references become read-only certificates with owned
immutable execution snapshots; arbitrary Secrets are never discovered.

The first valid certificate becomes the enabled default. Ayni runtime profiles
follow that default unless an explicit certificate is selected. Custom runtimes
have no certificate selection. Missing or unusable certificates block new signed
builds while connection setup remains available. Selection, default and key changes
retire current readiness; pending observations cannot reactivate a superseded
runtime, and admitted tasks retain their frozen images.

Reconciliation inspects the connected default revision before preparing public
trust changes. A scoped Kubernetes job validates the existing Ayni configuration
and regenerates the lock with the pinned CLI while preserving immutable inputs.
It receives no signing or registry-push material. Durable generation identities
and repository branches deduplicate draft trust-update PRs. After merge,
reconciliation checks the committed policy and lock, resolves or rebuilds the
signed image, and verifies the selected certificate before restoring readiness.
Only trusted environment builders receive signing seeds.


## Worker sandbox

The Linux worker requires fully enforced Landlock ABI 6 (kernel 6.12 or newer).
Both agent shell commands and Ayni quality execution run inside the same
unprivileged sandbox because repository tests are untrusted code. System and
toolchain paths are read-only; the workspace and disposable scratch paths are
writable. Worker output, repository/model credentials, frozen requests and the
launcher remain outside that authority. Signal and abstract Unix-socket scopes
prevent sandboxed children from signaling the parent or reaching its abstract
sockets. Unsupported or partial enforcement fails closed: there is no
unrestricted fallback. This needs no privileged container, added capabilities,
or special Docker/Kubernetes privilege.

The coordinator binds and retains final evidence outside the sandbox. Agent and
repository processes cannot rewrite worker progress, publication intent, final
evidence or receipts in the retained output directory. The Linux-only Landlock
dependency belongs to this private worker; it does not change Ayni upstream.

Runtime images are published at `ghcr.io/<source-owner>/<repository>`.
The selected GHCR connection supplies push and pull credentials; its authentication
account and connection-test namespace do not alter the runtime destination. One
backend helper derives build, pull, admission and read-only UI destinations.
Deployment-only fallback settings retain their existing layout.
Previously admitted tasks and pending Jobs retain their original frozen paths.


## Repository image lifecycle

Runtime preparation has one implementation: persisted control-plane intent,
pull-only registry discovery, isolated build/certification, automatic publication,
and a separate pull-only verification Job. A verified manifest digest, selected
runtime revision and source identity gate admission. A later selection cannot be
activated by a superseded Job. Tasks retain their frozen image digests.

Discovery uses `ayni-<arch>-<full-lock-hash>` on the approved repository path.
The source repository owns the image path. A tag is a
lookup key; only independently verified immutable manifest digests authorize work.
[Managed runtime execution](../guides/managed-runtime-execution.md) defines recovery and
credential boundaries. No GitHub workflow installation participates in readiness.

### Product overview reads and browser refresh

Repository and project overview pages consume control-plane summary queries.
Application readiness rules operate on complete operational-profile counts and
require readiness in every member repository. SQLite builds the read projection,
using the immutable task creation-time index for each project's latest task, then
retains the summaries in bounded pagination snapshots. This keeps overview meaning
out of browser components and avoids per-repository profile requests or sampling
global task/profile pages. Unavailable summaries produce a visible error instead
of an empty-success state. Resource detail and mutation APIs remain available.

Browser consumers share one notification stream, repair timer and in-flight
invalidation per resource scope. The page loaders declare their runtime, task,
connection, certificate or project dependencies. Background refresh pauses in
hidden tabs and while a dialog or editable input is active. Deferred stream
notifications and periodic repair recover changes after editing ends. Successful
mutations explicitly refresh their scope, bypassing this pause.

Connections, certificates and projects expose resumable resource streams with
an explicit snapshot-complete cursor. Their accepted lifecycle facts use the
transactional lifecycle journal/outbox alongside repository onboarding and source
synchronization.
Ownership-scoped Kubernetes metadata watches wake existing reconciliation lanes;
periodic repair and guarded readback remain active. See
[operator lifecycle facts and resource streams](../reference/operator-projections.md) for
authority, privacy, replay and recovery boundaries.


## Runtime preparation ownership

The application owns the durable preparation envelope, image phase vocabulary,
builder resolution state, workload settings and retained-credential interface.
Ayni-specific frozen requests, verification evidence and trust proposal data live
in the profile integration layer. Coordinators execute provider and Kubernetes
actions; SQLite persists the same typed state without importing coordinator types.
The public API remains a presentation boundary rather than the cursor format.

Each pending image preparation has one authoritative phase: discovery intent or
started discovery, build intent or started build, publication, publication
readback intent or started readback, verification, and activation. Intent is saved
before workload creation. Uncertain publication resumes registry readback before
another publication attempt. Activation retains verified evidence before cleanup,
and source/configuration guards prevent stale operations from restoring readiness.
Failure and retry budgets remain separate from execution phase. Cleanup progress
still uses the existing coordinator result boundary and never consumes a failure
budget. Coordinator errors remain separate from persisted execution outcomes.

Schema 10 decodes each retained schema-9 cursor and writes its typed equivalent in
one transaction, including builder lookup identity and timestamp. It preserves
operation names, frozen credentials, publication budgets and evidence; malformed
state aborts the transaction. Earlier binaries reject schema 10. This is an upgrade
contract, not a format suitable for rolling back to an earlier binary.

Credential retention is derived through one typed application interface from both
frozen preparation and pending execution. A credential rotation cannot delete a
revision while either stage can resume using it. Successful activation releases
both execution and preparation references after workload cleanup; artifact history
retains public provenance independently. Corrupt retained state blocks cleanup.

Private shared libraries own OCI syntax parsing, bounded GHCR credential decoding,
authentication request construction and registry response handling. API and domain
validation share syntax without depending on each other's admission policy. The
registry library receives only each caller's scoped credentials; it does not widen
probe, discovery, build or publication authority. Kubernetes discovery and
verification use a shared observer for owned Job/Pod completion and bounded result
collection. Execution adapters must implement every capability explicitly; missing
production behavior cannot inherit a successful default.


## Workspace and project ownership

Workspaces own shared repository membership and research/coding agent defaults.
Each project belongs to one workspace and selects a nonempty subset of its
repositories. It records a goal alongside its conversations and tasks. Repository
connections continue to own runtimes; workspace membership never copies a runtime.

Removing workspace membership preserves historical project selections and blocks
new work against the removed repository. New project selections must be a subset
of current workspace membership. Project defaults override workspace defaults;
omitted project settings inherit them.
Task admission freezes the effective selections, so later settings changes do not
rewrite admitted work.

### API and lifecycle

`ProjectService` exposes workspace CRUD and assigns a workspace when legacy project
creation omits one. Writes use durable idempotency
keys and resource versions for optimistic concurrency. Archival preserves projects,
conversations and task history while gating new project creation, research and
execution admission. Already admitted work retains its frozen inputs.

## Project conversations

The trusted operator API persists project conversations, research runs, task
proposals, versioned plans, approval receipts and linked outcomes. Workspace settings
select distinct research and coding connection/model defaults, with optional project overrides.
OpenAI is the enabled provider;
Anthropic and Bedrock cards confer no backend capability. The shared provider
library constructs Rig clients without importing server or worker policy.

A serialized conversation turn freezes model credentials, research inputs and its
own execution deadline. Consecutive turns reuse one resident planner Pod while
its credential binding remains compatible. A trusted checkout container prepares
fresh source for each turn using repository-scoped credentials; the research
container mounts source read-only and receives no checkout credentials or
Kubernetes API token. Its tools expose bounded listing, search and reading rather
than process execution. Repository instructions remain context, not authorization.
Different conversations can run concurrently.

SQLite retains the logical session identity, generation, active turn and idle
deadline. Workspaces default to 300 seconds and can select 86,400 seconds;
conversations can inherit or override either preset. Accepted user messages reset
activity, while replaying a receipt does not. Completion starts a fresh idle
period. Idle expiry does not interrupt queued or active turns, whose deadlines
and model-call budgets remain independent.

The control plane submits bounded turn envelopes and reads bounded receipts with
fixed Kubernetes exec commands. Turn identities include the session generation
and sequence. Worker-side serialization and retained receipts make repeated
delivery safe; SQLite accepts a result only for the current bound generation and
active run. Cancellation fences the run before stopping its owned Pod. A missing
active Pod interrupts its turn rather than repeating a potentially paid model
call. A later message creates a new generation using durable context.

Each new turn refreshes repository observations and source credentials. A changed
model Secret name, UID or revision requires stopping the old owned Pod before a
replacement generation can run. Source credentials travel only to the checkout
container; model credentials remain in the research container's read-only mount.
Pod adoption checks ownership and the expected specification, and cleanup uses
UID preconditions. The control plane's exec permission belongs to the trusted
namespace boundary; it is never delegated to model tools or worker service
accounts.

Research appends immutable plan content under one stable conversation plan ID.
Versions retain findings, scope, acceptance criteria, task proposal identities and
descriptive dependency/environment impact. A version's review state and failure
explanation can change; its researched content stays fixed. Lifecycle actions
(cancel, retry task and retry publication) remain separately confirmed action cards.
Worker updates do not propose additional work.

Approval names the exact plan ID and version. The current pending version admits
its entire task set in one SQLite transaction after revalidating project/workspace
membership, repository readiness and frozen source/runtime identity. No task is
admitted if any member fails validation. A changed source or runtime returns an
actionable explanation requiring refreshed research. The version decision and
admitted task receipts persist together; duplicate decisions retain original task
identities. Stale versions and conflicting concurrent decisions cannot replace the
current decision. Each run prepares at most one task per repository;
cross-repository dependency execution remains outside this contract. Accepted
attempt revisions project structured worker updates into the originating
conversation and safe archive, with revision fences and semantic deduplication.

SQLite owns the authoritative conversation state. Its append-only context archive
stores safe projections of messages, plans, approval decisions, proposals and worker
outcomes. Messages retain decisions, unresolved questions and research references
without model-generated classification. The conversation payload exposes only its
latest 16 messages; the scoped history API pages original records at a frozen
sequence watermark.

A versioned working index selects at most 16 extractive excerpts, prioritizing
plans, approvals and outcomes. It stores original record IDs and its source
watermark. Compaction commits with the archive update, checks the previous summary
version and cannot replace a newer watermark. It retains all original records;
this is bounded indexing, not lossless summarization or archive deletion.

Each research request freezes a scoped, credential-free selection of relevant older
records and recent context. Read-only worker tools search that selection or retrieve
an exact included record. They cannot query another project, expand repository
scope or read a live database. Partial selections explicitly report their archive
count and ask for a refined conversation question when missing context is needed.
A retained request without a selection remains readable and reports unavailable
retrieval until a new turn refreshes its inputs.

SQLite owns the authoritative archive independently of planner memory. A resumable full-state stream
updates browser cards, while periodic reconciliation recovers missed observations
and broker outages. Research result collection verifies the owned Pod and bounded,
generation-bound receipt before persistence. Idle cleanup preserves durable
records. Controller restart adopts surviving owned sessions and reconciles their
stored deadlines. Already admitted one-shot turns from older schemas finish with
their original frozen inputs; new turns use resident sessions.

## Agent profiles and capability enforcement

Workspace agent defaults optionally select separate planner and worker capability
profiles alongside their model/provider bindings. Project selections can reduce
tools, repository scope, model-call ceilings, deadlines and pinned workspace skills;
they cannot widen these. Project instructions append to workspace instructions.
The compiled role catalogs remain the upper authority, independently of metadata.

New message admission freezes both profiles into the research run. Preparation
uses those bindings even when defaults subsequently change, and freezes the exact
request budgets within deployment ceilings. Worker profiles are frozen again for
the approved task's single repository and explicit execution limits. Approval
rejects a changed worker profile or binding revision, requiring new research.
Ordinary retries retain their task snapshot. New dispatch rejects disabled or
unready connections; already dispatched executions retain their credential revision
until completion or explicit cancellation.

A content digest binds the role, builtin-role-v1 policy revision, provider/model,
connection revision, exact builtin and selected instructions, exact skill text and revision, tools and resource
scope. Profiles contain safe binding identifiers; credential values and Secret
names/UIDs remain outside profile/model context. Frozen request and run profiles
must match. Startup verifies revisions and bindings rather than adopting changed
settings into historical work.

The worker registers only allowed tools and checks the frozen role and scope again
inside each tool invocation. Read-only planner mounts and bounded retrieval remain
in force; it receives no command or repository mutation tool. Worker commands and
quality execution retain the existing Landlock sandbox, protected-path enforcement,
mandatory final checks and separate publication authority. Disabling an agent's
quality tool does not disable the worker's mandatory check.

The runtime contract defines ScopedToolAdapter as a future MCP attachment boundary.
An implementation must authorize every invocation against application-issued
capabilities. Tool descriptions, skill text and server metadata cannot grant a
capability. No external MCP server, marketplace or dynamic hierarchy is enabled.
