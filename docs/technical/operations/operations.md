# Operations

Runtime configuration sets `codingTurns` (default 24) and `qualityRepairLimit`
(default 3, accepted range 0–10). Admission freezes both limits in the task
snapshot. All phases share the frozen deadline. Worker admission requires the
pinned runtime digest and source identity; check failures are distinct from
unavailable infrastructure or evidence.

Task details display Progress, Quality and Pull request. Missing measurements
are unavailable. A published draft can have failed quality. Retry publication
resumes retained final work; retry task starts a new attempt with its frozen
snapshot. Persisted budgets apply after worker restart. Collection is idempotent
and resource cleanup follows durable collection.

## Preparation state upgrade

Schema 22 is the current database format. Recognized schemas 6–21 migrate in
order before reconciliation. It retains typed preparation, authenticated
verification/runtime-operation/coding inboxes, accepted decisions, outbox retry
and acknowledgement state, and operator lifecycle journal/outbox records.

The schema-9 to schema-10 migration converts retained preparation cursors in one
transaction while preserving frozen source/runtime identity, operation names,
evidence, credentials and remaining publication budget. Later migrations preserve
pending receipts and events. Malformed retained state aborts the upgrade; repair
from a trusted backup rather than deleting pending work.

Back up the database and run one control-plane writer during upgrade. Earlier
binaries may reject the upgraded schema. Rollback requires a compatible binary
or a quiesced backup restoration that reconciles external work performed since
the backup. Do not run an earlier binary against an unsupported schema.
After restart, uncertain publication still reads the registry before retrying, and
accepted runtime activation and its durable cleanup intent now precede workload
cleanup; cleanup completion cannot overwrite a replacement generation. Credential cleanup retains revisions
referenced by preparation as well as pending Jobs, then releases preparation
references once activation and workload cleanup complete.

## Workspace ownership upgrade

The schema-19 to schema-20 migration assigns each retained project a workspace,
using `workspace-<project-id>` as its stable identifier. Repository membership,
lifecycle state and timestamps carry over; agent defaults move to the workspace
and the project inherits them.

Existing project IDs, conversations, proposals, tasks and frozen execution inputs
stay intact. Existing projects have an empty goal until edited. The migration
creates no repository runtimes and reruns no work.

### Upgrade procedure

1. Back up and quiesce the database as described above.
2. Upgrade the control plane, generated clients, BFF and UI together.
3. Verify workspace membership and inherited agent settings after upgrading.
   Preserve retained state when creating workspaces.

Workspace writes use resource versions; refresh after a stale-write error.
`dev/ctl test workspaces CONTROL_PLANE_IMAGE WEB_IMAGE` exercises ownership
and restart persistence against the local Helm chart in an isolated Docker Desktop
namespace, with runtime execution disabled.

## Project plan upgrade

The schema-20 to schema-21 migration groups retained create-task proposals into
versioned conversation plans. It preserves proposal IDs, task and attempt links,
action receipts and admitted work. Cancel and retry actions stay explicit action
cards. Migration creates no tasks and reruns no research or worker execution.
Plans retain up to 512 versions within the conversation payload limit. If migrated
state exceeds a limit, the transaction aborts and preserves the schema-20 database;
restore from a trusted backup or repair the retained state rather than deleting
history.

Upgrade the control plane, generated clients, BFF and UI together after backing up
and quiescing the database. The legacy individual-proposal API remains available
for lifecycle action cards and receipt replay; new task creation requires approval
of the exact plan ID and current version. Retained proposal decisions preserve
their receipts rather than duplicating admission. Earlier binaries do not support
schema 21 and later; follow the rollback procedure above.

Plan approval revalidates all proposed tasks before committing any of them.
Source/runtime drift blocks approval with a refresh explanation; request
new research instead of retrying an outdated plan. Rejections and revision requests
retain the version history. Duplicate approvals return the existing receipt;
stale or conflicting decisions require refreshing the conversation.

`dev/ctl test plans CONTROL_PLANE_IMAGE WEB_IMAGE` exercises seeded approval replay,
stale/concurrent review decisions, rejected versions, migration and persistence
across control-plane restart on Docker Desktop. Runtime execution is disabled; the
fixture proves API and retained-state behavior without provider or worker calls.
Fresh atomic task admission is covered separately by SQLite repository tests; this
fixture does not bypass provider source observation or model credential checks.

## Conversation context upgrade

The schema-21 to schema-22 migration archives retained conversation messages,
plans, decisions and safe worker outcomes before reducing each conversation's
message projection to its latest 16 messages. It preserves task snapshots,
approval receipts, plan content and frozen research requests. Back up and quiesce
SQLite before upgrading the control plane, workers, generated clients, BFF and UI
together; older binaries cannot open schema 22.

The context archive is authoritative SQLite state on the control-plane volume.
It grows with accepted records and has no automatic deletion or retention policy.
The 100,000-record conversation limit stops new writes with an actionable capacity
error. A working index does not reclaim archive storage: operators must monitor
volume capacity and retain trusted backups. Starting a new conversation gives new
work its own capacity without deleting old evidence.

Context records contain at most 64 KiB. Research requests carry a bounded frozen
selection, and the worker reports when that selection omits archive records.
Plans, proposals and run metadata still share the conversation payload's 3 MiB
limit; plans retain at most 512 versions. Capacity errors leave accepted evidence
intact. A migration that cannot preserve retained state aborts; repair or restore
from a trusted backup instead of deleting history.

The working index contains at most 16 excerpts with source IDs and a sequence
watermark. Updating it neither deletes original records nor reclassifies every
historical decision or question. Its version and watermark protect against stale
compaction. Provider credential bindings, secret metadata, environment values and
raw task snapshots are excluded from context records and frozen retrieval tools.

`dev/ctl test context CONTROL_PLANE_IMAGE WEB_IMAGE` validates older-history search,
scoped pagination, approval and message replay, frozen worker retrieval, schema-21
migration and control-plane restart in a disposable Docker Desktop namespace.
Runtime execution is disabled. Synthetic retained state proves API and durability
behavior; it does not claim a live model or worker Job executed.

## Historical builder cutover

Use the explicit `docker-desktop` context for local deployment. Back up the private
database and stop obsolete global operations when crossing the schema-9 builder cutover. Upgrade server,
worker, generated clients, BFF and UI together. Migration retires obsolete global
settings and operations while preserving connections, certificates, registries,
runtime history, published images and retained tasks. Cleanup is limited to
recorded resources; never delete namespace-wide resources or retained data.

Use cleaned Helm values and `helm upgrade --install --reset-values`. Removed
fields fail schema validation. Builder identity now comes from the committed lock
and official compatibility catalog; engine configuration stays under
`runtime.managedRuntime.build`. Unsupported historical runtimes remain visible but
do not grant new admission. Existing tasks retain their frozen runtime references. A retained task admitted with
Ayni 0.14.5 may finish or resume using its original evidence contract; this narrow
historical parser does not permit new admission, onboarding, trust preparation or
builds with 0.14.5. Active repository compatibility remains Ayni 0.15.0.

The protobuf baseline is `repository-builders-v4`. The intentional removal of the
global Ayni service permits a single reset from `managed-images-v3`; subsequent PRs
retain breaking-change checks. Repository reads expose durable onboarding status,
reason and inspected commit. AnalyzeRepository requests analysis independently of
connection access status. Rebuild remains a separate explicit authorization.

Recovery snapshots are private operational state and are never automatically
published. Preserve final work and its evidence until publication recovery has
finished. A conflicting remote head requires operator attention and must never
be overwritten.

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

## Connection credential retention

Credential rotation retains previous Secret revisions while unfinished Jobs or
frozen runtime references still need them. Terminal task snapshots also retain
their old pull Secret because task and publication retries use the frozen
snapshot. This deliberately favors retry recovery over immediate credential
cleanup; do not manually delete those Secrets while retained work can be retried.

Removing an administrator connection from deployment configuration disables it
without deleting its Secret. Connection deletion checks current, pending and
retired revisions for consumers before deleting UI-managed Secrets. Labels
identify ownership; Kubernetes RBAC grants Secret access by namespace and
resource, not by these labels.

Runtime registry selections are additive protobuf metadata retained by the
schema-8 migration;
this UI refinement requires no database reset. Existing profiles without a runtime
selection inherit their repository’s legacy default until explicitly edited.
After every runtime has an explicit selection, that legacy default no longer
retains an unused registry connection. Frozen attempts and pending builds continue
to retain their credential revisions. Changing a runtime destination is a
version-checked, idempotent mutation and wakes reconciliation without restarting
the control plane.

Runtime registry selection now stores only the connection ID. Older duplicated
namespace protobuf fields are reserved and ignored when existing payloads are
read; the certificate migration preserves those payloads. Legacy
repository write APIs accept an omitted
namespace as inheritance, reject a supplied different namespace, and store the
connection's actual namespace as compatibility metadata. Runtime routing never
uses that legacy field. Reconciliation retires an old duplicated destination
before observing its frozen pending work; old Jobs finish at their original path
but cannot reactivate it. Newly admitted work waits for the normalized destination
to be verified.


## Certificate setup and rotation

Use **Certificates** to generate or import an Ed25519 signing key and its key ID.
The first valid certificate becomes the global default. Choose another enabled
default before disabling or deleting it; assigned certificates cannot be deleted.
Ayni runtime Edit dialogs can follow the default or explicitly select another
certificate. Projects inherit these repository runtime selections. Custom runtimes
receive no signing material.

Register administrator signing Secrets explicitly through
`connections.adminCertificates`. Legacy `build.signing_secret` references are
registered for compatibility and are deprecated for new setup. Updating a referenced
administrator Secret creates an owned immutable execution snapshot without changing
the source Secret. The UI displays public details and reports missing or invalid
sources. It never returns a private signing seed.

Selection changes, default changes and key rotation retire admission readiness and
propose draft public trust-policy/lock updates only for inspected Ayni repositories.
Review and merge the linked PR, then wait for image verification before admitting
new work. Closed PRs and failed lock jobs remain visible; use Retry after
repair to retry. Source changes and stale builds cannot activate a newer selection.

Trust preparation runs in the catalog-selected official builder digest. It receives
public certificates only. The worker injects its trusted launcher and checksum-pinned
mise into writable volumes. Lock updates must preserve immutable environment inputs.

Retain previous immutable signing Secret revisions while pending or frozen build
references need them. Already admitted tasks retain their frozen runtime image;
rotation does not rewrite their snapshots. Never delete administrator-owned source
Secrets as certificate cleanup. The supported schema-6 through schema-10 upgrade keeps
existing connections and retained execution state; back up the private database
before upgrading and preserve Secret ownership references.


## Isolated engine approval

A TLS Docker endpoint is authenticated transport, not proof of isolation.
`external-isolated` mode requires explicit approval of the engine boundary via
`build.isolated_engine`. Disposable engines require the configured local-development
or dedicated-builder mode and retain their scheduling and network constraints.
Never point external-engine settings at the Docker Desktop host/node daemon.
See [engine boundaries](../reference/repository-builders.md#engines-and-security).

Builder candidates are disposable engine state. No artifact PVC or manual
unpublished-image cleanup flow is needed. Normal Kubernetes image caching remains
kubelet-owned. Retry analysis and Rebuild never purge node caches or registry images.

See [repository-selected official builders](../reference/repository-builders.md) for immutable
version selection, attempt engines, security modes and recovery.

Opt-in verification receipt and safe delivery recovery are described in the
[verification event contract](../reference/verification-events.md). Keep reporting off until
the supervisor image, ingress/RBAC and transport boundary have been validated
together; receipt alone does not activate runtimes.

Verification reporting defaults to shadow authority. Set `runtime.verificationMode:
events` only with configured reporting transport; this selects new preparation
generations. Roll back future admission with `shadow`, keeping reporting transport
and original endpoints available while frozen event generations drain. Use
`python3 scripts/operations/verification-drain.py DATABASE` on the operator store
or quiesced backup to enumerate active and resumable dependencies. Existing
generations, including their retries, retain their original authority. Historical
reader/evidence retirement requires a separate audit; never delete retained PVCs
as a rollout shortcut. See [verification operations](../reference/verification-events.md).


## Compatibility drain inventory

Run `python3 scripts/operations/verification-drain.py DATABASE` against an
operator-owned SQLite database or quiesced backup. The tool opens the existing
database read-only and reads one transaction snapshot, without migrations or
cleanup. It supports schemas 13–18; an unsupported schema, missing table, malformed
cursor, unknown authority/phase/state, orphaned coding state or exceeded 10,000-row
inventory bound fails the command without producing a partial report. JSON cursor
size is bounded at 4 MiB. Reporting configuration and evidence bytes are not
validated by this inventory.

`reachable_generations` includes managed preparations, pending verification,
trust review continuations, official-builder operations and removed-repository
cleanup. Failed, paused and Pod-less continuations stay visible. Missing frozen
reporting selection counts as legacy; shadow verification still depends on the
legacy authoritative reader. The report shows retained verification and legacy
phase/builder reader dependencies and preserved legacy failure counters without printing frozen requests, credential
references, evidence bytes or trust diagnostics.

`coding_attempts` includes historical attempts as well as attempts with incomplete
cleanup, queued/running/input states or pending receipts. Schemas before 17 and
missing coding-admission rows count conservatively as legacy. Completed cleanup
does not remove a publication recovery dependency: the active succeeded/failed
attempt remains a `publication_recovery_candidate`. This deliberately overcounts
until the versioned application reader checks its frozen Protobuf result, commit,
publication outcome and deadline. It does not claim a retry is currently eligible.
Historical legacy attempt counts remain separate from live collector counts.

`retained_history_rows` counts runtime revisions/artifacts, attempt history,
resource events, page snapshots, action receipts and event journals/outboxes.
These counts identify retained representations to audit; they do not determine
which payloads use a legacy encoding. Check historical API/snapshot reads with a
compatible binary and inspect retained PVC recovery/evidence files, Kubernetes
workloads and provider readback separately. An empty SQLite inventory cannot
prove those external dependencies have drained.

The report always sets `historical_reader_and_evidence_audit_required: true` and
`compatibility_reader_removal_safe: false`. Zero live dependencies never authorizes
reader removal, retained-data deletion or a rollback to an incompatible schema.
Keep versioned historical readers and publication recovery until an explicit
migration and external evidence audit establish that they are unnecessary.

Verify the report with
`python3 -m unittest discover -s scripts/operations -p 'test_*.py'`.


## Project research and OpenAI connections

The deployment's `runtime.modelSecret` remains a compatibility source for retained
legacy attempts. It registers an administrator-managed OpenAI connection backed by an immutable
Secret snapshot. Source rotations validate a new snapshot before activation;
failed replacements keep the old working revision. Secret values are never
copied into SQLite. UI connections store write-only keys in
immutable, owned Secret revisions. Project settings select connection/model defaults
for research and coding independently. New runs and tasks freeze Secret identity;
rotation retains revisions needed by retained work, and a failed candidate keeps
the previous working revision. Disable blocks new admissions without rewriting
existing work. Credential management is trusted-operator only.

`runtime.projectAgentModelCalls` defaults to 24 (range 1–128), and
`runtime.projectAgentDeadlineSeconds` defaults to 600 (range 1–3600). Research uses
the pinned worker image in dedicated Jobs, without a ready coding runtime. The
research container mounts immutable source read-only; only checkout init containers
receive scoped source credentials. Results are bounded and accepted before cleanup.

The schema-18 to schema-19 migration adds conversations, immutable research requests,
action receipts and outcome-summary deduplication. Retained tasks and preparation
state keep their existing semantics. Back up state and upgrade server, worker and
clients together. An interrupted model interaction needs an explicit user retry;
reconciliation never silently restarts lost running research Jobs.
