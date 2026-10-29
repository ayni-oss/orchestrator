# Roadmap

Ayni Orchestrator is evolving into an open-source software factory for AI agents.
The next phase turns its technical foundations into a product people can use to
understand, authorize, observe and improve software delivery.

This roadmap owns agreed direction and milestone scope. Each bounded PR owns its
requirements, acceptance evidence and delivery status. Start fresh branches from
current `origin/alpha` and target `alpha`; users retain merge authority.

## Foundation phase archived

The M1–M8 backend implementation phase is closed as a planning phase. Its
[delivery roadmap](docs/technical/history/backend-foundation-roadmap.md) is archived
for provenance. Current architecture and operational contracts remain authoritative.
Archiving the plan changes neither supported behavior nor acceptance requirements.

Live additional-provider qualification and sustained production capacity remain
unmeasured in the recorded evidence. Confirm the final committed-source cumulative
M8 result before claiming local acceptance complete. The
[operational acceptance contract](docs/technical/operations/operational-acceptance.md)
continues to govern qualification; implementation completion does not establish
autonomous coding capability or production readiness.

## Product direction

The project is the durable home for intent, decisions, work and outcomes. Chat is
the first input surface. Future repository events, schedules, API requests and
agent inputs enter the same scoped workflow with explicit policy and authority.

Keep isolated conversations and workers, immutable plan approval, frozen source,
managed Ayni quality, durable receipts and recoverable draft publication. Execution,
quality and publication remain separate outcomes. Delivery ends at a reviewable
draft unless a user explicitly grants further authority.

The former console and diagnostic frontend are removed. The example UI has been
promoted into [apps/web](apps/web/README.md), the sole web application. Preserve its
shell, typography, themes and components while wiring each delivery slice to
canonical APIs. Do not introduce parallel products or alternate visual designs.
The current UI supports workspace/project creation, manual GitHub App setup,
GHCR/OpenAI credential configuration, workspace OpenAI model/budget selection,
shared repository onboarding and exact-commit static Ayni inspection. Full adapter
discovery, runtime preparation and full delivery journeys remain planned.
Historical browser acceptance does not qualify current source; each implemented
journey needs its own replacement coverage.

### Product surfaces

| Surface | User outcome |
| --- | --- |
| Workspace overview | Find attention items, compare project progress and understand shared capacity. |
| Project overview | Understand goals, decisions, approved work, blockers and delivered results without reading every chat. |
| Conversations | Discuss intent, inspect agent activity and review plans with their originating context. |
| Work graph | Inspect dependencies, frozen upstream selections and the path blocked by unfinished work. |
| Execution inspector | Follow each turn and attempt through timing, evidence, failures, recovery and draft publication. |

The interface explains the current stage, elapsed time, waiting reason and next
available action. Infrastructure details are available through inspection. A task
may finish coding and fail quality; a conversation may wait for a pod, a provider,
a tool or a human. These states must remain distinguishable.

### Durable events and responsive feedback

PostgreSQL remains the transactional workflow authority. Committed outbox delivery,
JetStream transport and idempotent consumers preserve accepted work across replay
and restarts. Browser updates use a scoped gateway with resumable observation.

Add incremental assistant output and live preparation, model, tool and task
activity. Distinguish provisional output from accepted results and reconcile after
reconnect. Token fragments need not become individual durable domain events.
Disconnecting a browser does not cancel accepted work.

Future event inputs retain origin, scope, deduplication identity and the applicable
policy revision. A trigger may initiate investigation or propose a plan within its
grant; receiving an event does not confer approval or execution authority. Begin
with a bounded repository-event journey before expanding trigger types.

### Traceability and measurement

Every delivery slice includes durable execution history, correlated traces and
metrics. Link the originating input, workspace, project, conversation, turn, plan,
task, attempt, provider call, tool invocation and workload generation where relevant.
Retries, restarts and recovery retain causal relationships; long workflows use
linked operation traces rather than one indefinitely open span.

Extend the existing OpenTelemetry and operational metrics foundations. Durable
history explains and recovers accepted work; traces diagnose execution; metrics
reveal aggregate behavior. Sampled traces and process counters cannot serve as
authoritative project accounting. Keep sensitive content out of exported telemetry.

Use these measurement definitions when specifying analytics contracts:

| Unit | Meaning |
| --- | --- |
| Turn | One admitted conversation input and its resulting agent execution. |
| Round | One model invocation and the resulting tool executions, when present. |
| Attempt | One authorized execution of a task. |
| Recovery | An explicitly linked continuation or repair with its own authority. |
| Delivery | A reviewable output with execution, quality and publication recorded separately. |

| System performance | Workspace and project understanding |
| --- | --- |
| Queue age and admission-to-start latency | Approval-to-draft lead time and work in progress |
| Pod scheduling, startup and source preparation | Time blocked by dependencies or human decisions |
| Active, idle and queued workers; resource use and restarts | First-attempt quality pass rate, eventual delivery and recovery frequency |
| Outbox age, broker backlog and redeliveries | Failures by execution stage and classified reason |
| Provider/tool latency, errors and uncertain effects | Turns, rounds, tool calls and tokens per delivered task |
| Cancellation latency, cleanup and restart adoption | Rework after review, when review outcomes are captured |

Define cohorts, denominators, time windows and unknown values for every rate.
Distinguish cancelled, blocked, failed and uncertain outcomes. Aggregate from
durable identities without counting transport replay as new work. Record cost
only when supported by explicit pricing or billing evidence, with its provenance.
Keep resource identifiers in scoped analytics and traces rather than unbounded
metric labels. Support drill-down from an aggregate to the underlying evidence.

## Product delivery sequence

These milestones describe planned outcomes. Each requires focused PRs and passing
evidence before completion; none is declared implemented by this roadmap.

### Next focus: repository preparation with a usable UI

PR 0 established a [dev-only live editing loop](dev/LIVE.md): frontend HMR, cached
Rust rebuild/rollout, explicit schema migration/reset and retained-state diagnostics.
Each product slice uses this loop during implementation and still requires normal
immutable-image deployment and acceptance evidence.

The first integration foundation adds the new app shell, PostgreSQL-owned named
connections and credential revisions, manual GitHub App validation and basic
workspace/project navigation. Hosting owns credential storage/backup encryption;
there is no application-level encryption. This is partial delivery of the first
catalog slice, not completed repository onboarding. Subsequent PRs add guided App
setup, further catalog lifecycle management, runtime registry permission handling and
combined live qualification. The configuration surface includes GHCR PAT entry,
OpenAI access/model discovery and workspace connection/model/budget selection;
selected OpenAI turns freeze credential revisions and use scoped runtime delivery. Each connection is selected explicitly; multiple named
connections per capability are supported. See
[connection foundation](docs/technical/architecture/integration-connections.md).


The shared source catalog adds GitHub installation/repository discovery, retained
connect operations and direct project associations, including reuse across workspaces.
Its source-access checks do not establish runtime readiness. Static Ayni inspection now freezes an exact commit and validates policy, lock,
locked input digests and public trust requirements using pinned Ayni APIs. Full
adapter discovery, registry/certificate setup, trust-change PRs, runtime production
and live GitHub qualification remain outstanding.

Deliver shared repository integration, Ayni inspection, verified runtime images
and project selection before expanding product analytics or event inputs. Build
the UI and canonical backend together in small end-to-end slices. Each slice must
be runnable locally and expose its actual states, evidence, blockers and next
actions; backend completion alone does not complete a slice.

Repository integrations live alongside workspaces. Projects associate directly
with catalog repositories, and projects in several workspaces can reuse the same
connection and verified runtime. Workspace repository membership is not required.
Association grants only the defined project use; it does not expose credentials
or another project's conversations. Task attempts freeze the repository identity,
source commit, lock and runtime artifact rather than following a moving selection.

| Slice | Maintainer can test through the UI |
| --- | --- |
| Shared catalog and project association | Configure source access, connect a repository, create a workspace/project and select the shared repository. Reuse it from a project in another workspace; inspect safe access failures and exact command receipts. |
| Frozen Ayni inspection and setup | Analyze committed policy and lock at an exact commit. Inspect compatibility, registry and certificate findings; finish configuration and review a narrowly authorized trust PR when required. |
| Verify an existing runtime | Discover an immutable image, inspect separate environment verification and see ready, missing, invalid and unavailable states without silently replacing an image. |
| Build, publish and recover | Enable bounded automatic preparation for a missing image, observe build/publication/verification/cleanup and explicitly authorize replacement of an invalid image. Resume after interruption without duplicate effects. |
| Project conversation to task delivery | Discuss, review and approve an exact plan, select verified repository/runtime evidence, authorize execution and inspect coding, managed quality and draft publication separately. No per-task terminal source preparation is required. |
| Feedback and cumulative qualification | Follow questions, failures and exact recovery back to the conversation, inspect the full causal chain and repeat the journey across reload, reconnect, concurrent tabs and owner restarts. |

The [repository preparation contract](docs/technical/architecture/repository-preparation.md)
defines gates, authority, shared ownership and frozen evidence. The
[delivery and acceptance plan](docs/technical/research/repository-runtime-workspace-delivery.md)
defines slice dependencies, UI coverage, failure cases and qualification limits.
Initial deployment and administrator Secret provisioning may require local tools;
the qualified product journey must handle repository setup and task preparation
without manual database changes, fabricated evidence or local source-pack commands.

### Wider product sequence

| Milestone | Observable outcome and acceptance |
| --- | --- |
| Repository preparation and product workspace | Deliver the next-focus slices above, including the shared catalog, project associations, Ayni setup, runtime production and project Chat, Plans and Tasks. Extend the sole example-derived application against canonical APIs; qualify every browser journey. |
| Observable execution | Add the execution inspector, live progress and incremental assistant text. Explain queue, preparation, model/tool execution, quality and publication stages with timing and evidence. Trace a full journey across API, broker, supervisor and worker, including interruption and replay. Instrument the first workspace slice rather than deferring telemetry to this milestone. |
| Browser-driven development | Complete the repository-preparation journey through trusted server-side source preparation, verified digest-pinned runtime selection and explicit budgets. Preserve exact approved source and upstream selections; add canonical APIs and browser coverage together. |
| Project and workspace insight | Ship overviews, attention, dependency visualization and scoped analytics using the measurement definitions above. Demonstrate drill-down, replay-safe counting, pagination and recovery-aware rates with known input cohorts. |
| Orchestrator self-development | Use a reviewed plan to deliver a real change to Orchestrator through the product, actual agent execution, managed Ayni quality and a retained reviewable draft. Capture human interventions, resource usage, limitations and recovery evidence. Deterministic coding fixtures alone do not satisfy this outcome. |
| Event-driven entry points | Add a bounded repository-event integration with inspectable trigger policy, origin, deduplication and approval boundaries. Duplicate delivery, reconnect and worker restart do not duplicate tasks or external effects. Design the input contract to admit schedules, API and agent inputs later. |
| Sustained operation and qualification | Run a defined realistic workload over a recorded duration, concurrency and hardware envelope. Measure throughput, latency, storage growth, saturation, recovery and provider behavior against explicit acceptance thresholds. Complete outstanding provider qualification and cumulative acceptance evidence. One successful self-development task does not establish sustained capacity. |

The first usable journey starts with shared repository setup and verified runtime
preparation, then project creation/association, discussion, exact plan approval,
execution, progress inspection, question/failure recovery and draft review.
Deliver it through small end-to-end slices. Use its measured behavior to refine
subsequent scope and capacity targets.

### Observable execution delivery

Retain accepted activity independently of seven-day watch replay cleanup and
expose stable, scoped history pages alongside live progress and provisional
assistant text. Correlate API admission, transport, supervisor, isolated worker,
provider/tool execution and task source, coding, quality and publication. Keep
recorded observations, provisional text, accepted outcomes and sampled diagnostics
distinct. The inspector includes task evidence, phase timing, waiting reasons and
explicit next actions, with approvals and recovery implemented against canonical
APIs alongside the next-focus UI slices.
See the [execution timeline contract](docs/technical/architecture/execution-timeline.md)
and [combined deployed acceptance](docs/technical/operations/observable-execution-acceptance.md).

### One product workspace

The example UI is the active application in `apps/web`; its duplicate source tree
and the standalone diagnostic frontend are removed. New repository, preparation,
conversation and delivery journeys extend this application and its existing design
system. Keep durable chat, plans, tasks, execution timelines, tracing and exact
receipts in the backend. Browser acceptance covers reload, reconnect, scope,
concurrent actions and interrupted-command retries against canonical APIs.

## Delivery discipline

- Define user outcomes, observable states, authority and failure/recovery cases before implementation.
- Extend canonical v1 contracts in place through alpha; keep generated consumers and documentation synchronized.
- Validate affected behavior on the explicit Docker Desktop Kubernetes topology and extend cumulative acceptance.
- Maintain one example-derived frontend; qualify each new journey against canonical APIs before advertising support.
- Keep system operation and project analytics scoped to the same workspace/project authority as their underlying records.
- Keep deployment choices explicit and self-hosting practical. Document dependencies, configuration, export and recovery without requiring a proprietary observability service.
- Record actual evidence and blocked checks in each PR. Release publication, merges and destructive cleanup retain their existing authorization boundaries.

## Further scope

Repository onboarding, required source/registry/certificate configuration and
runtime preparation are in the next focus above. Broader project/workspace
lifecycle management, provider administration, runtime fleet management and richer
operations views remain driven by those journeys. Specify capabilities against
current contracts before adding screens. Multi-user access and tenancy expansion
need separate scope and decisions.

Ayni remains an independently useful quality CLI and portable environment builder.
Its [independent roadmap](https://github.com/ayni-oss/ayni/blob/main/ROADMAP.md)
remains in its own repository.
