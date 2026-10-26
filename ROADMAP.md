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

The console and initial product web application are withdrawn. Preserve the former
`main` SvelteKit UI under [examples/web](examples/web/EXAMPLE.md) only as a design
reference. It is excluded from active builds and has no current backend wiring.
There is no supported browser UI for now. Continue the backend roadmap; design and
qualify a future product interface in a separate delivery decision. Historical
browser acceptance does not qualify current source, and combined profiles that
require it remain blocked until their replacement coverage exists.

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
| Product workspace | Design a future web shell and project Chat, Plans and Tasks surfaces using the preserved example as reference and current APIs as authority. Creation, discussion, exact plan approval, task inspection, recovery and publication preserve scope and exact retries across reload, reconnect and concurrent tabs. The console is removed; qualify each new browser journey before claiming support. |
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

### Observable execution delivery

Retain accepted activity independently of seven-day watch replay cleanup and
expose stable, scoped history pages alongside live progress and provisional
assistant text. Correlate API admission, transport, supervisor, isolated worker,
provider/tool execution and task source, coding, quality and publication. Keep
recorded observations, provisional text, accepted outcomes and sampled diagnostics
distinct. The inspector includes task evidence, phase timing, waiting reasons and
explicit next actions, with approvals and recovery implemented against canonical APIs when a future UI is authorized.
See the [execution timeline contract](docs/technical/architecture/execution-timeline.md)
and [combined deployed acceptance](docs/technical/operations/observable-execution-acceptance.md).

### UI reference and future workspace

The former app is an example only. The discarded console and initial workspace
are not a foundation for further UI work. Keep durable chat, plans, tasks,
execution timelines, tracing and exact receipts in the backend. Define the next
interface from user journeys and the example before implementing screens or
connecting legacy data access. Any future browser acceptance must cover reload,
reconnect, scope, concurrent actions and exact interrupted-command retries.

## Delivery discipline

- Define user outcomes, observable states, authority and failure/recovery cases before implementation.
- Extend canonical v1 contracts in place through alpha; keep generated consumers and documentation synchronized.
- Validate affected behavior on the explicit Docker Desktop Kubernetes topology and extend cumulative acceptance.
- Keep the legacy web example outside active builds; implement and qualify future UI workflows against canonical APIs before advertising browser support.
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
