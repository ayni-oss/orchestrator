# Control-plane event and flow inventory

Source snapshot: 11 October 2026, orchestrator commit `1f4e03a44e3217d6e3cf14b9848096e0ca03c8ac`.

The [architecture assessment](control-plane-events.md) develops the proposed event protocol from this inventory. Expected contracts in this document describe recommendations, not implemented behavior.

The current control plane combines persisted commands, Kubernetes observations, worker files, provider responses and resource-change streams. These signals have different authority. Application outcomes require validated evidence; infrastructure observations establish execution status and inform recovery.

This inventory covers the implemented orchestrator checkout. Schema values for Custom Runtime, waiting states and future capabilities do not establish an implemented flow. Separate coding, check and publication Pods, independent CI assessment and coordinated multi-repository tasks remain documented research.

## Signal levels

| Level | Producer | Meaning | Authority to advance |
| --- | --- | --- | --- |
| Command and intent | Operator, client or controller | Requested action plus frozen inputs and idempotency identity | Authorizes execution only after admission and durable intent |
| Infrastructure observation | Kubernetes adapter | Job/Pod existence, lifecycle, exit, restart, image identity, deletion | Authorizes scheduling, failure investigation, stop confirmation and resource reuse; does not establish task success |
| Activity and liveness | Trusted worker supervisor | Current stage, model/tool activity and diagnostic classification | Advisory display; future heartbeat indicates responsiveness, not progress or successful work |
| Work result and evidence | Trusted worker supervisor and operation workers | Candidate identity, verified runtime, quality artifact, final commit and delivery receipt | Can advance application state after control-plane validation |
| Provider observation | GitHub or registry adapter | Source head, registry manifest, reviewed merge, remote branch or draft identity | Establishes the external fact needed by the flow; worker narration cannot substitute |
| Accepted domain transition | Control plane and database | Validated state change tied to the current selection or attempt | Authorizes dependent actions and cleanup intent |
| Client projection | Resource streams and browser loaders | Current resource snapshots and user-visible status | Read-only presentation; not execution instructions |

An event describing a command is not proof that the command ran. A worker result is not accepted authority until identity, evidence and current controller intent match.

## Existing transports and durability

| Mechanism | Current contents | Durability and limitations |
| --- | --- | --- |
| SQLite `durable_actions` | Pending/completed/failed external action records and request replay receipts | Safe request payloads and idempotency checks; controller retries some external actions after restart |
| SQLite `resource_events` | Versioned protobuf resource snapshots and removal notifications | Appended with resource mutations in transactions; 4,096 events retained per service; bounded client replay, not permanent audit history |
| gRPC Watch APIs | Repositories, runtime profiles and one development task | Initial resource snapshots followed by cursor-based updates; database notification plus one-second wake-up fallback |
| SQLite connection/certificate tables and operation receipts | Credential references, public metadata, health and revisions | Persistent resources, but their saves do not append to `resource_events`; no corresponding Watch RPC |
| Kubernetes GET/LIST | Jobs, Pods, owned results, engines and Secret metadata | Runtime/coding loops nominally every five seconds; connection/onboarding/certificate loop every 30 seconds; no Kubernetes watcher runtime found |
| Container termination messages | Discovery JSON, candidate image ID, verification JSON, compact error codes, registry probe report | Coupled to Pod lifetime and Kubernetes message bounds; verification writer rejects reports above 4,096 bytes |
| Pod logs | `AYNI_TRUST_RESULT=` with public policy and lock output | Trust collector reads up to 2,100,000 bytes after Job completion; output is then validated and retained |
| Retained worker output volume | Progress, activity, result, usage, final work, publication journal and evidence | Coding output isolated by attempt directory; control plane polls files; no general worker-event ingestion RPC |
| Browser SSE and periodic reads | Task/resource updates, stream heartbeat, overview/detail refresh | Client connectivity and resource refresh; heartbeat every 15 seconds is unrelated to worker liveness |

Sources: [Periodic runtime, coding, connection and retention loops](<../backend/servers/control-plane/src/main.rs#L157>); [Transactional resource events and bounded retention](<../backend/servers/control-plane/src/adapters/sqlite/pagination_events.rs#L87>); [Resumable repository, profile and task streams](<../backend/servers/control-plane/src/adapters/grpc/watch.rs#L3>); [Database serialization and event notifications](<../backend/servers/control-plane/src/adapters/sqlite/database.rs#L63>); [Connection persistence and operation receipts](<../backend/servers/control-plane/src/adapters/sqlite/connections.rs#L26>); [Certificate persistence](<../backend/servers/control-plane/src/adapters/sqlite/certificates.rs#L3>); [Structured trust output collected from Pod logs](<../backend/servers/control-plane/crates/kubernetes/src/trust_lock.rs#L45>); [Verification reporting and the 4 KiB limit](<../backend/workers/runtime-worker/src/main.rs#L138>); [Task server-sent events and connection heartbeat](<../apps/web/src/routes/tasks/[taskId]/updates/+server.ts#L68>).

## Stored resource event kinds

These are the ten current append-event kinds. Most carry full resource snapshots, rather than one explicit business event per action.

| Service or flow | Exact kinds | Subscription |
| --- | --- | --- |
| Repository | `repository_connection`, `repository_snapshot`, `repository_removed` | `WatchRepositories` |
| Runtime | `runtime_profile`, `runtime_profile_revision`, `runtime_artifact`, `runtime_profile_removed` | `WatchRuntimeProfiles` |
| Development | `development_task`, `development_attempt` | `WatchDevelopmentTask` |
| Project | `project` | Stored, but no Project Watch RPC |

An attempt snapshot can contain new activity, progress, result, usage or receipt without an event type identifying which changed. Connections and certificates use separate persistent operation records. Provider model discovery uses an in-memory cache.

The companion [contract catalog](control-plane-event-contracts.csv) contains 289 source-indexed entries: 57 RPCs, 100 nonzero public enum values, 118 internal enum values, ten stored event kinds and four agent tools. State vocabularies are labelled as states, not claimed to be individually published events.

Sources: [Transactional resource events and bounded retention](<../backend/servers/control-plane/src/adapters/sqlite/pagination_events.rs#L87>); [Resumable repository, profile and task streams](<../backend/servers/control-plane/src/adapters/grpc/watch.rs#L3>); [Worker validation, state transitions and publication retry](<../backend/servers/control-plane/src/adapters/sqlite/coding_reconciliation.rs#L165>); [Project commands and resource events](<../backend/servers/control-plane/src/adapters/sqlite/projects.rs#L64>); [Model discovery cache and provider availability](<../backend/servers/control-plane/src/adapters/models.rs#L15>).

## Connection and certificate flows

| Flow and signals | Current progression gate | Persistence and cleanup | Expected event contract |
| --- | --- | --- | --- |
| GitHub App create, replace, enable, disable, test and manifest completion | Secret ownership/revision validation, App activation and provider tests | Candidate failure preserves a previously valid active revision; unusable active credentials disable authenticated work; retired revisions retained while referenced | Distinguish candidate validation, candidate rejection, active revision change and active availability |
| GHCR create, replace and test | Isolated probe reports authentication, push and pull outcomes | Persist report before deleting probe Job; cleanup of probe registry artifact is an independent outcome and can require retry without invalidating working credentials | Report each test outcome; separate registry-artifact cleanup from Kubernetes Job cleanup |
| Signing certificate create/import, replace, rename, enable, default change and delete | Key validation, immutable Secret revision and public identity | Explicit admin source snapshots and UI revision health are reconciled; retirement respects durable and live references | Public identity/revision/default/availability transitions only; selected trust and runtime verification are later flows |
| Credential retirement | No frozen preparation/task or live Pod still references revision | Delete owned Secret by identity; failed deletion retains the revision for future reconciliation | Cleanup requested/deferred/completed independent of the active connection's validity |

Certificate creation does not establish repository trust. A passed registry probe does not establish that a particular repository image exists or is verified.

Sources: [GitHub credential validation and rotation](<../backend/servers/control-plane/src/adapters/connections/reconciliation.rs#L145>); [Registry probe collection, activation and cleanup](<../backend/servers/control-plane/src/adapters/connections/registry.rs#L5>); [Registry probe transport and cleanup](<../backend/servers/control-plane/crates/kubernetes/src/registry_probe.rs#L153>); [Certificate health and administrator revision reconciliation](<../backend/servers/control-plane/src/adapters/certificates/admin.rs#L97>); [Durable image phases and retained credential references](<../backend/servers/control-plane/crates/application/src/runtime_preparation.rs#L47>).

## Repository and project flows

| Flow and signals | Current progression gate | Consequence and limitation |
| --- | --- | --- |
| Repository connect | Durable connect intent; GitHub repository discovery and identity/permission validation | Creates/transitions connection; optional Ayni enrollment; interrupted intent retried by onboarding loop |
| Inspect and analyze | Exact branch-head observation; immutable repository snapshot; policy and compatible lock inspection | Onboarding status becomes analyzing, enrolled or failed; existing runtime cursor `retry()` is called during enrollment |
| Repository source synchronization | Poll GitHub head and committed policy/lock; normalize platform, registry and signing identity | Frozen preparation generation and selected profile revision prevent old operation results activating new desired inputs |
| Repository or runtime registry selection; runtime certificate selection | Valid selected connection/certificate and version guards | New selection retires current admission authority; frozen in-flight work retains its original references |
| Project create/update/archive and membership | Repository membership and expected resource version | Transactional project update and event; active membership gates task admission/retry; no automatic multi-repository execution |
| Repository removal | No project references, no retained tasks and no active preparation | Atomically emit removals and retain failed-operation cleanup cursors; later controller cleanup can finish after resource removal |

`AnalyzeRepository` is currently more than observation: successful enrollment resets the runtime failure budget. This coupling should be visible as an explicit retry decision, or be removed from read/analysis behavior.

Sources: [Repository analysis and runtime retry reset](<../backend/servers/control-plane/src/coordinators/onboarding/mod.rs#L332>); [Official builder resolution and frozen preparation identity](<../backend/servers/control-plane/src/coordinators/reconciliation/official_builder.rs#L9>); [Repository removal and retained cleanup intents](<../backend/servers/control-plane/src/adapters/sqlite/repository_removal.rs#L201>); [Project commands and resource events](<../backend/servers/control-plane/src/adapters/sqlite/projects.rs#L64>); [Explicit runtime reconciliation actions](<../backend/servers/control-plane/src/coordinators/reconciliation/configuration.rs#L143>).

## Runtime preparation flows

The application persists nine image phases: `Discovering`, `DiscoveringStarted`, `Building`, `BuildingStarted`, `Publishing`, `PublicationReadback`, `PublicationReadbackStarted`, `Verifying`, `Activating`. Public display metadata adds states such as analyzing, resolving builder, cleaning up and ready.

| Step and observations | Current gate to next step | Evidence and cleanup ordering |
| --- | --- | --- |
| Freeze preparation inputs | Source head, policy/lock digests, contract version, platform, registry selection, certificate and workload settings | Generation retained before workload creation; stale pending work is discarded through cleanup |
| Resolve official builder | Discovery Job completion plus valid platform manifest digest for catalog builder | Save frozen builder identity before discovery cleanup; resolved identity can replay after restart |
| Prepare signing trust | Trust Job completion plus validated structured policy/lock result collected from logs | No-op result becomes verified and is saved before cleanup; changed files become a retained proposal |
| Review signing trust | Provider draft/open/closed/merged observations plus policy checks at observed commit | Deterministic owned branch and retained proposal deduplicate PR work; reviewed merge still requires current policy revalidation |
| Discover repository image | Successful lookup worker returns `Pending`, `Missing`, `Resolved` or `Failed` | Authenticated missing image permits build; access failure does not; save next phase/result before cleaning discovery Job |
| Build/certify candidate | Engine continuity, successful Job status and valid SHA-256 candidate ID | Candidate saved before publication; engine retains candidate state through publication, so it cannot be cleaned immediately after build |
| Publish candidate | Publisher Job missing/running/succeeded/failed; compact candidate-missing classification | All terminal or uncertain publisher outcomes advance to independent registry readback; save readback intent before publisher cleanup |
| Read back publication | Registry digest and image ID match expected candidate | A publisher success does not grant readiness; mismatch is tag drift or explicit overwrite/rebuild policy; bounded publication budget |
| Verify pulled image | Job success plus owned Pod, successful verify container, image digest and JSON report | Validate certificate selection, source revision, lock digest, runtime fingerprint, platform and supported Ayni version |
| Activate runtime | Valid evidence, current source head and current selection/generation | Save binding/report and `Activating`; cleanup operation resources; then guarded artifact/profile activation; cleanup currently gates Ready |

Runtime verification executes a full `ayni check`, but `admit_runtime` does not require a passing quality outcome or zero check exit code. Runtime trust and immutable bindings authorize readiness; source quality is separately represented in the report. An unavailable or malformed artifact can still make the verification operation fail.

Sources: [Official builder resolution and frozen preparation identity](<../backend/servers/control-plane/src/coordinators/reconciliation/official_builder.rs#L9>); [Trust preparation, result validation and review workflow](<../backend/servers/control-plane/src/coordinators/reconciliation/trust.rs#L318>); [Structured trust output collected from Pod logs](<../backend/servers/control-plane/crates/kubernetes/src/trust_lock.rs#L45>); [Runtime preparation, failure counting, publication and cleanup](<../backend/servers/control-plane/src/coordinators/reconciliation/preparation.rs#L13>); [Evidence retention, cleanup and guarded activation ordering](<../backend/servers/control-plane/src/coordinators/reconciliation/activation.rs#L11>); [Runtime evidence admission checks](<../backend/servers/control-plane/src/profiles/ayni/evidence_policy.rs#L21>); [Discovery and compact build result observation](<../backend/servers/control-plane/crates/kubernetes/src/discovery.rs#L78>); [Verification observation and foreground resource cleanup](<../backend/servers/control-plane/crates/kubernetes/src/execution.rs#L166>); [Ayni invocation, runtime verification and quality summaries](<../backend/workers/runtime-worker/src/profiles/ayni.rs#L99>); [Durable image phases and retained credential references](<../backend/servers/control-plane/crates/application/src/runtime_preparation.rs#L47>).

## Kubernetes observation detail

| Current observation | Present interpretation | Information to preserve in a richer contract |
| --- | --- | --- |
| Job absent | Missing, start/adopt or lost-execution recovery depending on durable state | Expected operation identity, last observation and confirmed deletion versus observation unavailable |
| Job exists without terminal counters/conditions | Running | Scheduling, init progress, image pulling, worker startup and active execution as separate facts |
| Job failed counter or failed condition | Failed | Deadline, init failure, OOM, eviction or worker exit, with safe bounded diagnostics |
| Job succeeded | Start collecting operation result | Result availability and acceptance remain separate from process exit |
| Verify container termination | Exit code, message and image ID inspected after owned-Pod lookup | Worker reporting status, compact result reference and physical execution identity |
| Build engine Pod | Existence, deletion, terminal phase and restart count | Engine continuity for candidate state; availability is not task progress |
| Foreground deletion and coding stopped check | Resource cleanup pending/completed and associated Pod terminal phases | Deletion requested, execution stopped, cleanup completed and any retained diagnostic evidence |

The current runtime adapter does not subscribe to Kubernetes Event resources or expose scheduling, image-pull and OOM details as first-class lifecycle events. These are candidate infrastructure diagnostics, not application outcomes. Status and ownership checks must remain the authoritative basis; diagnostic Event resources can supplement them.

## Runtime failure and retry flow

| Signal | Current handling | Required distinction |
| --- | --- | --- |
| Ordinary error returned by runtime reconciliation, including observation/API errors | Increment cursor failures, persist generic reason, schedule backoff; at three failures mark pending failed and cleanup | Failed observation, failed execution and failed evidence acceptance need separate outcomes and budgets |
| Repeated observation of the same failed Job | Can increment failure count repeatedly without creating a fresh workload | Failure count is not necessarily worker attempt count |
| `runtime_cleanup_pending` | Resume cleanup and retry without consuming runtime failure budget | Preserve this separation for every cleanup error category |
| Explicit runtime reconcile/rebuild | Durable action resets failure budget; rebuild or failed preparation may invalidate old pending source identity | Keep retry verification, retry publication, retry trust and rebuild as distinct requested actions |
| Successful repository analysis | Calls cursor `retry()` on existing profiles | Do not make hidden analysis side effects the only recovery path |
| Display `next-retry` | Always projected from cursor `next_at`, even when failures have exhausted retries | Show retry eligibility, exhaustion and action required explicitly |

Sources: [Runtime preparation, failure counting, publication and cleanup](<../backend/servers/control-plane/src/coordinators/reconciliation/preparation.rs#L13>); [Retry metadata and retry reset](<../backend/servers/control-plane/src/profiles/ayni/preparation_state.rs#L22>); [Observation failures, concurrency and runtime budget](<../backend/servers/control-plane/src/coordinators/reconciliation/mod.rs#L133>); [Explicit runtime reconciliation actions](<../backend/servers/control-plane/src/coordinators/reconciliation/configuration.rs#L143>).

## Task admission and execution

| Flow and signals | Current gate | Outcome and resource consequence |
| --- | --- | --- |
| Create task | Active project membership, operational Ready profile, verified matching artifact and source bindings | Freeze source, runtime revision/digest, model, constraints, budgets and absolute deadline; persist task, attempt and events |
| Schedule coding | Queued attempt with absent Job | Create deterministic immutable specification and Kubernetes Job; transition attempt to Running after creation |
| Infrastructure running | Job present without terminal result | Accept advisory coding snapshots (legacy attempts poll activity/progress) and refresh credentials; pending and active execution are largely collapsed into Running |
| Live worker progress | Accepted retained snapshot for event generations; bounded `progress.json` for legacy | Save worker phase, consumed turns, repair round, latest full check and publication state; budget counters cannot regress |
| Live activity | Accepted safe activity snapshot for event generations; bounded `activity.json` for legacy | Save advisory activity on attempt; identical snapshots do not generate another update |
| Job succeeded | Legacy result collection or event-mode retained-envelope recovery | Read and validate result, usage and evidence; success state comes from result.execution, not Kubernetes exit alone |
| Job failed | Try final result first, then retained recovery, otherwise mark failure | Failure fallback uses last activity or generic diagnostic; result may establish application outcome even when Job failed |
| Job missing after Running | Retained recovery plus unexpired deadline | Stop old resources before recreating same logical attempt; no recovery means lost-execution failure |
| Final result accepted | Artifact hash/scope, commit, receipt destination and frozen budget checks | Transaction updates attempt and active task plus resource events; then cleanup and coding-done marker |

The worker returns normally after writing many failed application outcomes. A Kubernetes-successful worker can therefore report `Failed`, `NoChanges`, `InputRequired` or failed publication. Conversely, a failed Job can still have a valid retained result.

New event-mode coding generations accept authenticated retained terminal reports while the Job is running. Legacy generations retain completion-triggered file collection. Execution stop remains a separate replacement gate; see [coding lifecycle reporting](coding-events.md).

Sources: [Task admission and frozen identities](<../backend/servers/control-plane/crates/application/src/tasks.rs#L150>); [Coding result collection, recovery and cleanup](<../backend/servers/control-plane/src/coordinators/reconciliation/coding.rs#L25>); [Decisions from task state and Kubernetes execution state](<../backend/servers/control-plane/crates/application/src/reconciliation.rs#L18>); [Worker validation, state transitions and publication retry](<../backend/servers/control-plane/src/adapters/sqlite/coding_reconciliation.rs#L165>); [Worker execution and independent result outcomes](<../backend/workers/runtime-worker/src/coding/mod.rs#L27>); [Worker progress and result contracts](<../backend/libs/runtime/contracts/src/coding.rs#L41>).

## Worker code actions and quality

| Worker flow | Current signals and retained state | What permits progress |
| --- | --- | --- |
| Admission and checkout | Attestation, sandbox probe, frozen source checkout and protected-input checks | Valid runtime/source and enforced sandbox before coding tools |
| Coding and repair | Model hooks; generic tool start/finish; session state; turn usage before provider request | Shared frozen turn budget, available time and validated constraints |
| Agent tools | `run_command`, `ayni_verify`, `ayni_check`, `commit_changes` | Local worker owns sequencing; control plane does not issue each tool invocation |
| Focused verification | Requested-scope Ayni artifact | Advisory feedback for coding; cannot satisfy full repository completion |
| Mandatory full quality | `Checking`, latest full-check evidence and measurement outcomes | Worker always runs full check; unavailable checks get bounded operational retries |
| Quality repair | `Repairing`, repair round, retained prior context and shared coding budget | Actionable failed quality, remaining repair/turn/time limits; operational failure is not automatically a source defect |
| Changes made by checks | Commit actual permitted changes and rerun check | Final evidence must match resulting commit; repeated source mutation becomes unavailable evidence |
| Finalization | Immutable signals, digest, source fingerprint, final commit and retained Markdown | Evidence binding and constraint validation; missing evidence cannot pass |
| Input required | Bounded requested question in final result | Maps to WaitingForInput and execution cleanup; no dedicated answer/resume RPC exists in current service schema |

Quality can be Passed, Failed or Unavailable independently of execution and publication. Valid eligible work can be published with failed/unavailable quality, disclosed in the draft.

Activity uses these 17 stages: Request, Checkout, Build, Install, RuntimeVerification, OutputPreparation, ModelCredentials, ModelExecution, ModelRequest, ModelResponse, ToolStarted, ToolFinished, Checkpoint, Finalization, ReportWrite, Completed. They do not include dedicated quality-check or publication milestones; those currently appear through progress phases and files.

The activity constructor enables file retention only for `code`. Noncoding operations emit tracing activity but do not provide the same retained activity stream. Activity sequence starts with the process-local log, so future replay needs a separate execution-generation identity when the same attempt resumes.

Sources: [Worker execution and independent result outcomes](<../backend/workers/runtime-worker/src/coding/mod.rs#L27>); [Mandatory quality checks, repair and final evidence](<../backend/workers/runtime-worker/src/coding/quality.rs#L68>); [Atomic recovery generations and budget preservation](<../backend/workers/runtime-worker/src/coding/recovery.rs#L37>); [Bounded activity snapshots and hooks](<../backend/workers/runtime-worker/src/coding/activity.rs#L42>); [Advisory activity and bounded failure vocabulary](<../backend/libs/runtime/contracts/src/activity.rs#L8>); [Worker progress and result contracts](<../backend/libs/runtime/contracts/src/coding.rs#L41>); [Verification reporting and the 4 KiB limit](<../backend/workers/runtime-worker/src/main.rs#L138>); [Ayni invocation, runtime verification and quality summaries](<../backend/workers/runtime-worker/src/profiles/ayni.rs#L99>).

## Publication and model usage

| Flow | Authoritative evidence | Current handling |
| --- | --- | --- |
| Final work retained | `final.json`, source recovery and bound quality evidence | Publication can resume without another coding session |
| Push intent | `publication.json` identifies task/attempt/repository/branch/base/head | Persist intent before remote write; deterministic attempt branch and remote-head checks protect adoption |
| Draft created or adopted | Matching provider PR receipt including exact head and marker | Persist receipt before quality comment; uncertain response recovers through provider readback; conflicting head stops delivery |
| Quality comment | Bound report and persisted comment digest | Retry searches exact report content/marker to avoid duplicate comment; explicitly truncates large comment content |
| Publication-only retry | Retained commit/result plus publication eligibility and unexpired deadline | Requeues same attempt, retains budgets and final work, clears coding-done marker; does not start fresh coding |
| Ordinary task retry | Failed/DeadlineExceeded task, remaining attempt count, unexpired task deadline and project membership | New attempt retaining frozen inputs; distinct from same-attempt recovery and publication retry |
| Model usage | Per-interaction request/completion/failure records outside sandbox | Collect supported usage and completion status; missing measurements remain unavailable, not zero |

Worker publication is local worker coordination, not a sequence of separate control-plane Pods. A stream should expose accepted milestones without silently moving publication credentials or authority into the controller.

Sources: [Worker validation, state transitions and publication retry](<../backend/servers/control-plane/src/adapters/sqlite/coding_reconciliation.rs#L165>); [Publication intent, provider readback and receipts](<../backend/workers/runtime-worker/src/coding/publication.rs#L281>); [Atomic recovery generations and budget preservation](<../backend/workers/runtime-worker/src/coding/recovery.rs#L37>); [Durable model usage accounting](<../backend/workers/runtime-worker/src/coding/model_usage.rs#L26>).

## Cancellation and cleanup

| Flow | Current ordering | Expected separate facts |
| --- | --- | --- |
| Explicit task cancellation | Persist task/attempt cancellation first; later coding reconciliation cleans execution | Requested cancellation, terminal decision, stop confirmation and cleanup completion |
| Deadline expiry | Periodic database reconciliation expires eligible task and nonterminal active attempt | Deadline is authoritative even if worker results arrive late; retain observed late side effects separately |
| Terminal task cleanup | Preserve available model usage, delete Job/request/credentials/engine, confirm associated Pods stopped, mark coding-done | Cleanup should not rewrite execution/quality/publication outcomes |
| Same-attempt recovery | Retained source/budgets plus deadline; cleanup and stop confirmation before restarting | A physical execution generation must finish before another may write the same retained state or publish |
| Runtime activation cleanup | Save verified evidence before deletion; delete all operation resources; activate | Existing sequence is restartable, but Ready is coupled to cleanup completion |
| Engine loss | Observe Pod continuity/restarts; cleanup old durable identity; create new build identity within recovery budget | Engine availability is insufficient proof that build made progress; candidate state must not be reused across engine loss |
| Repository removal cleanup | Delete logical configuration with cleanup cursor retained; clean remaining workloads later | Logical removal and physical cleanup can already complete separately |
| Retired credential cleanup | Check durable and live references before owned deletion | Consequential cleanup cannot delete credentials still needed by frozen work |

Cleanup is usually a consequence of an accepted result or terminal controller decision. It remains a prerequisite where old execution must stop before replacement, credentials must stay available, or retained engine state is needed by publication. Cancellation and deadline enforcement cannot wait indefinitely for a cooperative worker result.

Sources: [Evidence retention, cleanup and guarded activation ordering](<../backend/servers/control-plane/src/coordinators/reconciliation/activation.rs#L11>); [Verification observation and foreground resource cleanup](<../backend/servers/control-plane/crates/kubernetes/src/execution.rs#L166>); [Coding result collection, recovery and cleanup](<../backend/servers/control-plane/src/coordinators/reconciliation/coding.rs#L25>); [Decisions from task state and Kubernetes execution state](<../backend/servers/control-plane/crates/application/src/reconciliation.rs#L18>); [Deadline transitions and resource events](<../backend/servers/control-plane/src/adapters/sqlite/task_lifecycle.rs#L163>); [Repository removal and retained cleanup intents](<../backend/servers/control-plane/src/adapters/sqlite/repository_removal.rs#L201>); [Durable image phases and retained credential references](<../backend/servers/control-plane/crates/application/src/runtime_preparation.rs#L47>).

## Client and control-plane health signals

Task SSE carries `task` and `related` events. The current component listens to task changes and heartbeat/error events, but not `related`; activity-only attempt updates rely on periodic refresh. Browser refresh is suppressed in hidden tabs and while editing, with later reads catching changes.

The SSE heartbeat establishes that the stream endpoint is responsive. It says nothing about Pod connectivity, worker heartbeat age or task progress. Worker activity records occur on actions and are not a periodic heartbeat.

Model discovery has a 15-minute cache, 30-second failure backoff and ten-second request timeout. Background-loop exit and database failure are supervised and can stop the server. Operational health and metrics are observations, not task completion evidence.

Sources: [Task server-sent events and connection heartbeat](<../apps/web/src/routes/tasks/[taskId]/updates/+server.ts#L68>); [Browser stream listeners and invalidation](<../apps/web/src/lib/components/task-update-status.svelte#L30>); [Shared browser polling and refresh suppression](<../apps/web/src/lib/refresh.ts#L8>); [Model discovery cache and provider availability](<../backend/servers/control-plane/src/adapters/models.rs#L15>); [Background-loop supervision and shutdown](<../backend/servers/control-plane/src/process/supervision.rs#L38>).

## Confirmed gaps and consequences

| Gap in current code | Consequence | Needed contract |
| --- | --- | --- |
| Verification report includes quality findings but writer caps entire JSON at 4 KiB | Reporting can fail after valid runtime verification | Compact durable result envelope plus full evidence reference; separate reporting outcome |
| Verification errors collapse to a short generic reason; command output is normally suppressed | Runtime identity, CLI artifact, transport and reporting failures become hard to distinguish | Stage-specific typed errors with safe retained diagnostic references |
| Three failed reconciliation passes exhaust budget | Outages or repeated observation of one Job can look like three execution attempts | Separate observation retries, physical attempts and evidence rejection budgets |
| Retry timestamp survives exhaustion | UI promises work the controller will not perform | Explicit eligibility, exhaustion, next attempt and required action |
| Readiness follows cleanup completion | A cleanup backlog can delay already verified admission | Evaluate independent verified/admission state and cleanup state; preserve current-selection guards |
| Final coding collection waits for terminal Kubernetes state | A final durable worker result is not sufficient to trigger acceptance while execution remains Running | Accept immutable final result under a generation fence; stop/cleanup tracked independently |
| Different flows report through logs, termination messages and files | Uneven information and different restart/retention properties | Shared lifecycle envelope, with operation-specific evidence payloads |
| Activity/progress read or parse errors are silently ignored in normal collection | Missing, malformed and unchanged progress look alike | Explicit reporting health without falsely failing otherwise valid work |
| Attempt `related` SSE updates have no component listener | Activity and usage may wait for periodic refresh | Subscribe/invalidate attempt changes directly |
| Pending/init/image-pull failures collapse into broad running or failed status | Operators lack an explanation for work that never reached the worker | Persist bounded Kubernetes startup and termination diagnostics |
| No worker heartbeat or fenced stream generation | Connection health cannot establish worker liveness; old/restarted producers are difficult to order | Per-execution identity, heartbeat and stop/lease policy |
| Resource replay is count-bounded snapshots | It cannot serve as a complete audit trail or explicit causal history | Define lifecycle event retention separately from resource projection replay |
| Connections/certificates lack resource-event publication | Their UI uses polling and safe metadata reads | Add domain notifications if those flows join the event architecture |

These are source-level findings. They do not identify the historical verification attempt's root cause without retained runtime diagnostics.

## Existing validation and remaining proof

Existing tests cover resumable resource watches across restart, cancellation precedence, frozen retry inputs, duplicate result collection, full-evidence requirements, budget preservation, uncertain publication recovery, stale selection rejection and asynchronous cleanup. Their presence supports the intended invariants; this research did not rerun them or exercise a live cluster.

A new event contract needs failure-injection tests for report publication before exit, exit before report, duplicate delivery, broker outage, malformed evidence, stale execution, result/cancellation races, external success with lost response, disk loss and blocked cleanup. It must retain each accepted outcome and diagnostic independently of Pod deletion.

Sources: [Existing replay, evidence and lifecycle tests](<../backend/servers/control-plane/src/adapters/sqlite/tests/recovery.rs#L450>); [Existing restart and cancellation tests](<../backend/servers/control-plane/src/adapters/grpc/task_flow_tests/lifecycle.rs#L79>); [Decisions from task state and Kubernetes execution state](<../backend/servers/control-plane/crates/application/src/reconciliation.rs#L18>); [Atomic recovery generations and budget preservation](<../backend/workers/runtime-worker/src/coding/recovery.rs#L37>).
