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

Restore useful Svelte layouts and components from the former web application.
Rebuild its data access against current contracts; exact legacy feature parity is
not the objective. Project configuration and management screens follow real product
journeys rather than the former service inventory.

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

| Milestone | Observable outcome and acceptance |
| --- | --- |
| Product workspace | Restore the web shell and project Chat, Plans and Tasks surfaces against current APIs. Creation, discussion, exact plan approval, task inspection, recovery and publication preserve scope and exact retries across reload, reconnect and concurrent tabs. Retain the console until replacement coverage passes. |
| Observable execution | Add the execution inspector, live progress and incremental assistant text. Explain queue, preparation, model/tool execution, quality and publication stages with timing and evidence. Trace a full journey across API, broker, supervisor and worker, including interruption and replay. Instrument the first workspace slice rather than deferring telemetry to this milestone. |
| Browser-driven development | An approved plan reaches execution through trusted repository/source preparation, digest-pinned runtime selection and explicit budgets without terminal steps. Server-side verification preserves frozen source and upstream selections. Review required onboarding and configuration support for this journey; add canonical APIs where missing. |
| Project and workspace insight | Ship overviews, attention, dependency visualization and scoped analytics using the measurement definitions above. Demonstrate drill-down, replay-safe counting, pagination and recovery-aware rates with known input cohorts. |
| Orchestrator self-development | Use a reviewed plan to deliver a real change to Orchestrator through the product, actual agent execution, managed Ayni quality and a retained reviewable draft. Capture human interventions, resource usage, limitations and recovery evidence. Deterministic coding fixtures alone do not satisfy this outcome. |
| Event-driven entry points | Add a bounded repository-event integration with inspectable trigger policy, origin, deduplication and approval boundaries. Duplicate delivery, reconnect and worker restart do not duplicate tasks or external effects. Design the input contract to admit schedules, API and agent inputs later. |
| Sustained operation and qualification | Run a defined realistic workload over a recorded duration, concurrency and hardware envelope. Measure throughput, latency, storage growth, saturation, recovery and provider behavior against explicit acceptance thresholds. Complete outstanding provider qualification and cumulative acceptance evidence. One successful self-development task does not establish sustained capacity. |

The first usable journey is project creation, discussion, plan review, explicit
approval, execution, progress inspection, question/failure recovery and draft
review. Deliver it through small end-to-end slices. Use its measured behavior to
refine subsequent scope and capacity targets.

### First observable execution slice

Retain accepted conversation activity independently of seven-day watch replay
cleanup and expose stable, scoped history pages in the workspace inspector.
Show current activity live and offer explicit refresh for a new history boundary.
Preserve recorded turn identities and related plan/task resource references;
accepted timestamps describe observations, not exact stage durations. Detailed
worker progress, round/tool timing, incremental assistant text, task-specific
inspection and complete cross-service tracing remain subsequent slices. See the
[execution timeline contract](docs/technical/architecture/execution-timeline.md).

### First product workspace slice

The first slice restores workspace/project navigation and conversation chat in a
Svelte application served by the existing local Rust gateway. It includes resource
creation, retained transcript and older history, resumable observation, explicit
turn cancellation, current activity, provider usage and exact interrupted-command
retries. Message and turn counts come from canonical retained facts.

Plans, task evidence, recovery, publication and agent configuration remain available
through a scoped contributor-console link. The console stays supported until those
journeys have equivalent web acceptance. Incremental assistant text, full tracing,
round/task analytics, event triggers and browser-driven source preparation remain
subsequent slices. PR evidence must distinguish browser fixtures from actual
PostgreSQL/JetStream and isolated-session execution.

## Delivery discipline

- Define user outcomes, observable states, authority and failure/recovery cases before implementation.
- Extend canonical v1 contracts in place through alpha; keep generated consumers and documentation synchronized.
- Validate affected behavior on the explicit Docker Desktop Kubernetes topology and extend cumulative acceptance.
- Replace console workflows only after equivalent web acceptance passes; then remove obsolete entry points and update local startup guidance.
- Keep system operation and project analytics scoped to the same workspace/project authority as their underlying records.
- Keep deployment choices explicit and self-hosting practical. Document dependencies, configuration, export and recovery without requiring a proprietary observability service.
- Record actual evidence and blocked checks in each PR. Release publication, merges and destructive cleanup retain their existing authorization boundaries.

## Further scope

Project/workspace lifecycle management, repository onboarding, provider connections,
runtime management and richer operations views are candidates driven by these
journeys. Specify required capabilities against current contracts before adding
screens. Multi-user access and tenancy expansion need separate scope and decisions.

Ayni remains an independently useful quality CLI and portable environment builder.
Its [independent roadmap](https://github.com/ayni-oss/ayni/blob/main/ROADMAP.md)
remains in its own repository.
