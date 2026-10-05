# Architecture

Ayni Orchestrator has **one orchestration lifecycle with separate interfaces and
runtime profiles**. The product journey is **Connectors → Projects → Tasks**.
[Decision 0009](decisions/0009-connectors-projects-and-internal-assessments.md)
defines the agreed product; [Decision 0010](decisions/0010-orchestration-interface-boundaries.md)
defines the code boundaries. These decisions describe the target, not a claim
that every product flow is implemented.

The project foundation provides named projects with shared editable repository membership,
project-owned tasks targeting one member, and archive history. The
remaining connector, project-readiness, environment-production, and internal
assessment flows are target behavior; current implementation ownership and gaps
are tracked in [the product epic](https://github.com/ayni-oss/orchestrator/issues/90).

## Three interfaces, different responsibilities

| Interface | Direction and contract | Owns |
| --- | --- | --- |
| Native gRPC | Inbound, versioned user contract in `proto/ayni/orchestrator/v1` | Authentication, capabilities, request bounds, protobuf conversion, status mapping, pagination and watch transport |
| Server HTTP | Inbound, server-specific operational and browser integration endpoints | Liveness, readiness, metrics; protected provider callbacks and one-time credential exchange when those flows are implemented |
| Kubernetes | Outbound, internal execution adapter using the Kubernetes API | Deterministic Jobs and related resources, isolation, credential references, scheduling, observation and cleanup |

The gRPC API is the product API. The server-side web BFF adapts it to browser
forms and sessions; it does not implement another lifecycle. HTTP callbacks
must enter the same application use cases after protocol-specific validation.
Their bounded secret-bearing bodies must not enter logs, caches or ordinary
resource responses. Operational readiness is distinct from a project's
readiness to admit a task: connector configuration must remain possible when
execution infrastructure is unavailable.

Kubernetes Jobs are disposable execution resources. They are not user-facing
product resources or a second database. Durable application state decides
admission, retries, cancellation, readiness and publication. Job completion is
an observation, not proof that an environment or candidate is valid. This
design does not require product CRDs or expose Kubernetes objects through gRPC.

## Shared orchestration and profile policy

The core owns connector/project identity, repository membership, immutable
profile revisions, task admission, attempts, deadlines, budgets, cancellation,
candidates, evidence binding, publication and reconciliation. A project has
one or more repository connections, shared with other projects. A connector can serve many projects.

Both profiles use the same task and candidate lifecycle, persistence,
Kubernetes execution adapter and provider publication path. A profile adds
explicit admission and verification policy; absence of Ayni metadata must
never be interpreted as permission to skip a managed check.

| Shared responsibility | Ayni Managed policy | Custom Runtime policy |
| --- | --- | --- |
| Admit an immutable OCI digest and platform | Verify committed Ayni inputs, producer trust and certificate through supported Ayni contracts | Validate the supplied container against the supported launcher, platform and isolation contract |
| Resolve the execution environment | Discover, or internally build a genuinely missing certified environment | Resolve and verify the owner's supplied image; no Ayni build or signing requirement |
| Develop and checkpoint immutable candidates | Make Ayni available as development feedback | Run the repository's supported native tooling without requiring Ayni |
| Bind assessments and evidence to the exact candidate | Independent internal Ayni verifier and bounded repair policy | Explicit custom assessment policy; do not label unassessed work as Ayni-verified |
| Publish and reconcile review objects | Present bounded Ayni findings and actual final outcome | Present the actual custom assessment or unassessed outcome |

Ayni owns lock interpretation, policy evaluation and certificate verification.
Orchestrator owns execution identity and verifies that observations and evidence
belong to the operation being reconciled. Shared scheduling and publication
must not acquire mandatory Ayni lock, signal or GitHub workflow fields.

## Product flow and trust

Connector setup validates a GitHub App and persists its operational credentials
with authenticated encryption. The encryption root remains in an existing
operator-managed namespace Secret. Registry pull/push, producer signing,
build-engine and model credentials remain deployment Secret references, not
connector records. Installation discovery can refresh without recreating a
connector. Disabling one prevents new provider operations and degrades affected
projects without deleting history.

Project inspection is tied to an observed default-branch commit. For Ayni
Managed it checks access, committed inputs, compatibility, producer trust,
owned integration files and the environment. Missing Ayni inputs require owner
action. An idempotent setup PR changes only owned integration files when needed;
the target does not install GitHub build or assessment workflows. Reinspect
after merge before advancing readiness.

Environment discovery uses the deterministic identity and private
`ghcr.io/<owner>/<repository>-ayni` package. Only confirmed absence authorizes a
deduplicated internal build intent. Access errors, invalid signatures, drift
and uncertain publication require remediation or reconciliation. Independent
verification binds the platform-specific manifest digest before admission.
Source-only changes can reuse the same environment after verification.

Task admission freezes repository/source, profile revision, digest/platform,
constraints, deadline and model budget. The agent develops in that image and
saves a checkpoint. The target Ayni verifier uses a fresh checkout of that
candidate in the same image, retaining evidence outside candidate-writable
storage. Findings allow at most three repair cycles within the original limits;
every repaired commit needs a new assessment. Candidate policy/lock changes
that invalidate the frozen environment require owner review rather than
automatic replacement certification.

Evidence binds project/task/attempt, base and candidate commits, verifier Job,
profile revision, runtime digest, lock/configuration, platform, Ayni version and
artifact digests. Valid work may become a draft with passed, failed, incomplete
or environment-update-required status. Cancellation blocks subsequent automatic
publication; no changes or no valid candidate produce no PR. Changed owned PR
heads invalidate earlier assessment authority. Reassessment does not silently
start another agent repair session.

Publication delivery uses validated full Git revision types for its frozen and
provider-returned heads. Execution port errors carry typed categories while the
adapter mapping preserves existing bounded diagnostic codes. Unknown failure
codes remain explicitly unclassified rather than implying retry authority.
Coding dispatch matches the closed profile kinds and rejects missing/unknown
wire values; Custom Runtime remains unsupported by this coordinator.

External mutations have durable intent and deterministic identities before
execution. After an uncertain response, reconcile existing Job, registry or
provider state before retrying. Preserve task history and frozen digests across
profile changes. Coding workloads receive no Kubernetes token, host socket,
controller database, signing key or registry-push credential. Checkout and
model credentials have separate workload scope; the verifier has neither model
nor signing/push credentials.

## Code structure

`backend/servers/control-plane` owns the server lifecycle and its private Rust
library crates. `backend/workers/runtime-worker` owns executable work. Only
contracts and behavior needed by both belong under `backend/libs`; being a Rust
library alone does not make code shared.

| Location | Current ownership |
| --- | --- |
| `proto/`, `backend/servers/control-plane/crates/grpc-contract`, `clients/typescript` | Public gRPC schema, generated bindings and wire syntax validation |
| `backend/servers/control-plane/crates/application` | Transport-independent automation and task admission/cancellation/retry and deadline eligibility, runtime selection/activation/observation, publication, coding reconciliation, assessment admission and readiness commands, lifecycle states, immutable bindings and atomic persistence ports; execution trait and neutral workload observations/settings |
| `backend/servers/control-plane/crates/kubernetes` | Implements execution operations with deterministic isolated Jobs, submission/adoption, observation and cleanup |
| `backend/servers/control-plane/src/adapters/grpc` | User-facing transport, capability enforcement, conversions, pagination and durable watch delivery |
| `backend/servers/control-plane/src/adapters/http` | Operational HTTP and projected health |
| `backend/servers/control-plane/src/adapters/sqlite` | Database schema, explicit protobuf persistence codec, transactional resources/actions/events; remaining lifecycle methods awaiting extraction |
| `backend/servers/control-plane/src/adapters/github`, `adapters/models.rs` | Provider communication and model discovery |
| `backend/servers/control-plane/src/coordinators` | Shared server state, onboarding and reconciliation coordinators; remaining protobuf/provider coupling is explicit |
| `backend/servers/control-plane/src/process` | Listener/runtime configuration, metrics and tracked background work with bounded shutdown |
| `backend/servers/control-plane/src/profiles/ayni` | Managed environment and repository preparation policy |
| `backend/libs/domain` | Shared change constraints, scope bounds, usage values and content digests |
| `backend/libs/runtime/contracts` | Neutral execution/coding inputs, activity, candidate reports, OCI/launcher admission and bounded input reading |
| `backend/libs/runtime/profiles/ayni` | Shared managed expectations/results, identity validation and release metadata |
| `backend/libs/integrations/github-actions` | Existing Ayni Actions assessment evidence contract shared by controller and worker |
| `backend/workers/runtime-worker` | Trusted launcher, generic checkout/process helpers and coding; worker `profiles/ayni` owns managed verification/build invocation, and `integrations/github_actions` owns Actions operations |
| `packages/bff`, `apps/web` | Server-side browser adapter and UI |

The root Cargo workspace retains one lockfile, toolchain and lint configuration.
Normal Rust module trees match the control-plane directories. Dependencies point
from concrete adapters to the contracts they implement: application cannot
import gRPC, SQLite, Kubernetes or executable packages. Kubernetes implements the
application execution port and cannot import Ayni profile policy or the public
API. Shared runtime contracts cannot import Ayni, Kubernetes, gRPC or executables.
The worker cannot import controller application code or the public API. The Ayni
dependency policy enforces crate direction.

Task admission and cancellation replay persisted request identities before reevaluating current
readiness. New admissions freeze source/runtime bindings through neutral values,
then atomically persist the task, initial attempt, idempotency record and both
events. The wire adapter and persistence codec preserve the existing protobuf
representation and action-hash format. Deadline durations and absolute timestamps
are distinct application types. Cancellation decisions use neutral lifecycle states;
the persistence adapter commits the action and task/unfinished-attempt changes
together, retaining completed attempts and their versions.

Explicit task retry is application-owned. Replay precedes current project, lifecycle,
attempt-limit and deadline checks. The persistence port atomically commits the new
attempt, frozen snapshot, task update, action receipt and both events. Deadline
eligibility uses the same application timestamp comparison with nanosecond precision;
expiration preserves terminal attempts and commits eligible task/attempt events together.
Onboarding coordination returns typed bounded failures; gRPC alone maps them to status
codes. Managed lock compatibility and trust-policy observations belong to the controller
Ayni profile and do not establish certificate verification.

Automation admission replays retained request identities before reevaluating the
current profile or workflow template. The managed profile validates its setup
files without credentials or mutations. SQLite checks observed profile/repository
versions and atomically commits the installation, pending action and event.
Provider setup work runs later through the onboarding coordinator.

All control-plane persistence, including native gRPC, onboarding and reconciliation,
uses a bounded 64-request database queue serviced by a dedicated thread that
exclusively owns the SQLite store. Accepted work finishes even if a caller
disconnects; retries replay its durable identity. Event notifications are emitted
after the operation changes the committed event watermark. Watches register
before reading their cursor, then await notification with a periodic recovery
poll; notifications never replace durable event authority. Repository onboarding, setup actions, deadlines, retention, runtime execution,
coding, assessment import, publication and readiness all use this queue.
Provider and Kubernetes requests execute outside database operations. The server supervises database-thread
termination and closes the queue during shutdown, draining accepted operations
within the bounded shutdown period.

Publication admission and automatic scheduling use application-owned commands
and persistence ports. Identical retries replay before reevaluating candidate
availability. New intents freeze candidate version and Git identity, and commit
the change request, provider intent and event atomically. Automatic scheduling
uses the frozen task publication request and excludes cancelled tasks and active
attempts. Provider-side recovery uses an application-owned provider port: it observes existing
reviews before allowing a push/create sequence and rejects mismatched frozen Git
identities. Uncertain observations stop mutations; an uncertain create response
is recovered by observing the existing review on retry. GitHub response mapping
and bounded bundle transport remain in the provider adapter. Bundle preparation
runs outside async executor threads. The integration coordinator still selects
durable intents and maps retained wire resources.

Readiness observation, authorization and confirmation are application-owned use
cases with closed policy/state/source values. They load current durable facts,
preserve newer status when historical evidence arrives, and observe uncertain
provider transitions without repeating them. Authorization persists intent before
provider mutation; confirmation rejects changed heads or assessment identities.
SQLite maps the existing protobuf representation and atomically persists status,
actions and events, while provider checks remain in the integration coordinator.

Runtime revision selection and activation use application-owned commands and persistence
ports. Identical source/policy observations replay the current selection; changes
and reverts select a new immutable revision. Activation rejects superseded revisions,
stale source/evidence bindings and conflicting digests. Matching ready-artifact replay
adds no events. SQLite checks the observed profile version before persisting and retains
atomic artifact/profile events and existing payloads. Managed report conversion,
frozen report authority, candidate policy changes and provider conclusion interpretation
belong to the control-plane Ayni profile; the coordinator gathers facts and runs adapters.

Runtime observation decisions and bounded retry backoff also live in the application
crate. Changed source/configuration discards pending execution before build or verification
handling. Lost or failed build acknowledgements trigger registry observation rather than
another build. Verification preserves its existing deadline and retained-build recovery
rules. Coordinators execute the decisions through adapters and persist the existing cursor.

Coding reconciliation uses application-owned attempt states and decisions. A
missing queued execution may start; a missing running execution is a failure and
cannot silently restart. Interruption cleanup precedes profile handling and late
success. Completed output is collected before cleanup. Controller output reads,
bundle hashing and assessment archive unpacking run off async executor threads.

Provider command deadlines cover request writes, response reads and process
exit. Request and response bytes are bounded, including retained policy input;
Actions source hashing runs off executor threads. Coding command deadlines also
bound output collection after the command exits.

Worker subprocesses own Unix process groups. Cancellation, deadlines and normal
leader exit terminate remaining group members. Verification/build invocation is
async; finite checkout, Git, candidate freezing and checkpoint work use bounded
blocking operations. Checkpoint serialization survives caller cancellation until
the accepted operation completes. Blocking Git drains stdout/stderr concurrently
and has a command deadline, preserving bounded diagnostics.

Kubernetes completion supplies bounded raw output; managed evidence interpretation
remains with the controller. Server reconciliation loops and finite model warmup
are tracked. Unexpected task failure stops the server; shutdown signals periodic
loops, allows in-flight work to finish and aborts work exceeding the bounded grace
period. Durable intents remain responsible for recovering uncertain external work.

Managed requests instantiate the generic execution/coding contract with a
required `RuntimeExpectation`. This preserves the serialized managed request
shape and fail-closed validation. A typed custom request can use its own
admission data without importing Ayni. That separation alone does not enable
custom execution in the current reconciler or worker.

## Implementation limits and next boundaries

The current managed path still uses repository-first onboarding and Actions
assessment collection. Its assessment contract requires a PR and workflow/run
identity; it must not be reused as the pre-publication internal verifier
contract. The connector, project readiness, internal environment production and
repair flows above remain tracked by [the product epic](https://github.com/ayni-oss/orchestrator/issues/90).
The project foundation is implemented with shared editable multi-repository membership and
project-owned tasks; connector management and project readiness remain follow-up
work. [Custom Runtime #66](https://github.com/ayni-oss/orchestrator/issues/66)
remains unimplemented end to end.

Assessment admission also uses an application-owned command and persistence port.
It replays matching immutable evidence before reevaluating profile membership,
rejects conflicting manifests and foreign repository profiles, and propagates
unavailable or corrupt replay state instead of treating it as absence. SQLite
retains the existing Actions-specific payload codec and commits assessment and
evidence events together. Profile evidence verification remains separate from
this persistence admission boundary.

The application crate owns the extracted lifecycle decisions and ports. Server
coordinators gather adapter facts and execute those decisions; they still read
generated protobuf resources and call concrete providers. SQLite retains the
explicit protobuf codec and transactional resource operations. These integration
layers are not a fully isolated domain model. Connector operations, project
inspection and internal environment/assessment production need their own vertical
commands and ports as those separately tracked product flows are implemented.
Closed profile dispatch rejects unsupported custom execution today.
Coding, build and verification Jobs already share neutral workload
construction rather than deriving one operation from another's manifest.

The Kubernetes adapter receives only execution settings. Application scheduling,
publication policy and managed GHCR source derivation live in the control plane.
Runtime JSON retains its strict format and is mapped explicitly into adapter
inputs. The digest-pinned `runtime.workerImage` chart setting is the only worker
image setting; the namespace comes from the Helm release.

The supported deployment remains Docker Desktop, one trusted team, loopback
access and one controller replica. This structural change does not alter the
protobuf contract, persisted schema, managed JSON format or rendered workload
isolation. Durable architecture belongs here; acceptance and delivery status
belong in the linked issues.

## Project-centered UI

[Decision 0011](decisions/0011-project-centered-ui-and-onboarding.md) amends the
membership, initialization and registry portions of Decision 0009. The UI uses a
responsive shadcn-svelte sidebar with Projects and Configuration. Project creation
uses Formsnap/Superforms validation and a searchable checkbox picker. Membership
rows retain removed repositories to validate historical task ownership; only the
project's current members can admit new work or retries. Readiness/profile resources
remain shared at the repository connection boundary. Schema version 4 rejects
older storage without modifying it.

Automatic inspection, approved bootstrap initialization, user-managed registry
credentials and sequential multi-profile tasks remain delivery targets. Current
runtime selection is still one operational, ready profile per task. Unavailable
summary data is presented explicitly; partial lists do not imply authoritative
latest-task results or complete readiness.
