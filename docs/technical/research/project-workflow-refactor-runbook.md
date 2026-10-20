# Project workflow refactor runbook

Date: 2026-10-19. Status: research procedure; target stack selected, detailed boundaries remain under investigation.

## What this establishes

- Design the target from product scenarios, authority, and recovery requirements before mapping existing components into it.
- Treat project conversations as the working interface for planning, execution feedback, and confirmed recovery.
- Investigate distributed event producers and a shared JetStream backbone, including project-agent consumption of task activity.
- Derive module, library, service, storage, and API boundaries from the resulting contracts.
- Use the [design worksheets](project-workflow-design-worksheets.md) to produce reviewable evidence at each gate.

This runbook defines a method, not implementation approval or a delivery-status tracker. Use [the roadmap](../../../ROADMAP.md) for direction and milestone scope; record delivery work and acceptance evidence in PRs targeting `alpha` without requiring separate GitHub issues. Record architectural decisions in focused decision documents after review.

## Confirmed product constraints

| Topic | Direction confirmed by the maintainer |
| --- | --- |
| Workspace | Defines reachable repositories, agent configuration, instructions, tools, and MCP integrations. Cross-project visibility includes objective and status with a description. |
| Project | Groups a chunk of work, conversations, plans, and task activity. The user decides completion with agent summarization. |
| Conversation isolation | Sessions share agent configuration but keep working context isolated. Fresh conversations receive objective and factual project status only, excluding other conversations' transcripts, plans, and reasoning. Cross-conversation memory is deferred. |
| Plan ownership | Cross-conversation editing has no demonstrated value. Investigate conversation-owned plans; do not silently import earlier plans into fresh research. |
| Concurrent work | Multiple plans and same-repository tasks may run concurrently in isolated worktrees with separate branches and draft PRs. |
| Plan and task | Plans organize changes; tasks execute through runtimes built with Ayni. Results feed subsequent planning. |
| Compatibility | Development state is disposable; breaking schema and API changes are acceptable. |
| Tenancy | Multiple tenancy is part of the broader system. Deployment tenancy and user management are outside this refactor's scope. |
| Recovery | A task reports its failure or question in its originating conversation, also visible in project-wide activity. |
| Authorization | The agent prepares a recovery proposal; the engineer must confirm it before recovery executes. |
| Continuation | Reuse retained work where safe. The distinction between resuming an attempt and creating a new attempt requires design. |
| Task boundaries | Read-only research/planning execution is separate from task execution. One repository per task is the preferred security boundary, subject to research. |
| Dependencies | Task dependencies are required. B can use an explicitly selected commit from A before merge; later upstream changes cannot silently alter B. DAG scheduling and task communication require research. |
| Completion feedback | Update project facts and assess meaningful outcomes automatically; the agent may proactively propose work. Proposals do not authorize execution. |
| Waiting and steering | Waiting for input retains durable work even if compute expires. Steering active execution is desirable, not an initial requirement. |
| Delivery | The current delivery responsibility ends at draft PR creation. Retained work has an explicit retention policy. |
| Capability order | Workspace instructions first, MCP integrations second, multiple providers third. |
| Proving ground | Pass basic sandbox scenarios before using Orchestrator to develop Orchestrator. Current sandbox failures must be reproduced and classified. |
| Responsiveness | Acknowledge requests promptly and expose actual preparation/model/tool activity. Avoid unexplained waits; the M1 contract contains proposed numeric targets that require maintainer approval before they become delivery gates. |
| Development strategy | Use a separate branch in this repository for now. Keeping the existing app usable throughout is not required; repository separation remains deferred. |
| Target stack | PostgreSQL owns transactional state; JetStream carries durable messages; gRPC provides commands and watches. Detailed component boundaries remain open. |
| Capacity | Active-project, conversation, and task counts have not yet been specified. |

Disposable state permits a clean replacement design. It does not authorize deleting a running deployment or retained external work during research. Preserve existing access boundaries and scope identities without introducing a new tenancy or identity platform.

## Why start with behavior

Event types encode decisions about ownership and authority. Choosing streams or microservices first would embed those decisions before we understand task recovery and project continuity.

The earlier source review found live-update suppression, coarse planner progress, and potential scheduling unfairness. Those are investigation leads, not a complete diagnosis of the latest main branch or a prescribed target architecture.

## How it works today

- The repository already has workspace defaults, versioned plans, task attempts, frozen runtime inputs, and evidence handling.
- Conversation context, returned worker outcomes, and resource watches provide foundations for continuity.
- The inspected JetStream path publishes accepted facts; planner execution and browser updates use other paths.
- Re-read current code and tests at a recorded revision before carrying any finding into a decision.

## Execute the investigation

Work through the gates in order. Complete independent inventories together, but do not finalize downstream contracts while their upstream decisions remain unresolved.

| Gate | Work and required output | Depends on | Exit criterion |
| --- | --- | --- | --- |
| A: Scope and scenarios | Confirm constraints; write working-day scenarios and measurable experience targets. | Maintainer answers | Every scenario states what the engineer sees and can do, including failure. |
| B: Risk evidence | Extract reusable mechanisms and failure lessons from representative prior flows without inheriting their schemas, states, compatibility, data, tests, or topology. Capture the target dependency and authority maps. | A | Every retained lesson has a source or reproducible observation, target implications are explicit, and no legacy behavior becomes an unstated requirement. |
| C: Domain and recovery | Define ownership, vocabulary, state machines, invariants, and recovery approval semantics. | A, B | Walk the failure-and-continuation scenario without ambiguous authority or state. |
| D: Contracts | Design event families, consumer behavior, gRPC commands and watches, and persistence boundaries. | C | Duplicate, late, missing, and replayed delivery have specified outcomes. |
| E: Components | Assign logical owners; compare modules, shared libraries, deployable services, and worker processes. | C, D | Every contract and side effect has one accountable owner; deployment splits have evidence. |
| F: Feasibility | Run bounded protocol, recovery, streaming, and scheduling experiments. | D, E | Evidence resolves the highest-risk assumptions or sends their decisions back for revision. |
| G: Implementation plan | Map target components to keep, replace, retire, or adapt; define cutover and removal. | F | Each slice has a visible outcome, dependency, acceptance tests, and rollback boundary. |
| H: Delivery | Implement reviewed slices and exercise them on Docker Desktop Kubernetes. | G | The complete feedback/recovery loop meets its acceptance criteria under failure. |

At each gate, record findings, alternatives, decision owner, unresolved questions, and the evidence required to proceed. The engineer prepares the analysis; the maintainer resolves product choices. Do not repeatedly seek confirmation for routine investigation already authorized.

## Anchor scenarios

Use these scenarios throughout modeling, experiments, and acceptance. Include a timeline of user-visible state, durable state, agent activity, and external effects for each.

1. Two conversations share configured capabilities but keep working context isolated. Fresh research does not inherit earlier plan assumptions; project activity still reports both outcomes.
2. An approved plan starts tasks. Progress remains visible while the engineer drafts a message or navigates elsewhere.
3. A task cannot continue. Its originating conversation explains the cause, retained work, and the decision needed; project activity shows the same unresolved item.
4. The engineer answers. The agent proposes recovery, including changed inputs and budget. Confirmation resumes safely or creates an explicitly linked attempt.
5. Two browser tabs confirm the same recovery. Only one execution starts; both converge on the same receipt.
6. A worker or planner disappears, an event is duplicated, or a result arrives after cancellation. No stale writer advances accepted state.
7. Results change an assumption. The project records outcome facts and the originating conversation proposes a plan revision without rewriting approved content or injecting it into other conversations.
8. The browser reconnects and the planner wakes after idle expiry. Both catch up within their scopes without losing accepted activity or replaying external actions.
9. Two tasks use separate worktrees of one repository. A dependent task receives a defined prerequisite artifact or commit; neither sees another task's uncommitted filesystem.
10. The user reviews an agent summary and explicitly completes the project. Draft delivery does not imply merge, deployment, or project completion.

## First implementation slice

The [roadmap](../../../ROADMAP.md#project-workflow-replacement) defines eight delivery milestones, separate from this runbook's research gates. M1 is a runnable PostgreSQL, JetStream, and gRPC slice with a scripted executor, durable admission, and resumable updates. Its [durable foundation contract](project-workflow-m1-durable-foundation.md) specifies the reviewable scope, authority, module map, local stack, and acceptance cases that runtime implementation must satisfy.

The replacement's cumulative acceptance harness is its only executable baseline.
Historical sandbox behavior supplies lessons, not compatibility requirements. Before
a runtime PR, complete the M1 contract's bounded PostgreSQL, authenticated isolated
JetStream/capacity, executor-handoff, watch/replay, and private-v1 reset experiments.
Runtime slice 1 deletes the superseded workflow code and tests as it installs the
replacement boundary. Extend the harness through planning, real execution,
dependencies, and confirmed recovery. M6 proves the core feedback/recovery loop; M8
establishes operational acceptance for the initial scope.

Use Orchestrator development as the first real workload only after replacement
sandbox scenarios pass. Self-development must not be the only mechanism available
to repair the system. Approved responsiveness and capacity targets remain regression
requirements for subsequent slices.

## Cutover and completion

The replacement is a deliberate reset in disposable development state. No deployed
user, retained workflow data, or old execution must be migrated or drained.

- Runtime slice 1 deletes superseded workflow transports, schemas, tests, flags,
  controllers, and documentation before registering the replacement boundary.
- The rollback point is before the first replacement PostgreSQL admission. Before
  that point, revert the slice; after it, reset the disposable replacement namespace
  rather than restoring a legacy authority or translating data.
- Late legacy results have no replacement authority and are rejected by the removed
  listener/consumer boundary.
- Regenerate contracts and managed locks through repository tooling, preserve Ayni's
  independent product boundary, and carry no compatibility shims into later milestones.

Completion requires passing scenario and failure tests, accurate architecture documents, dependency enforcement, and removal of obsolete paths. Report blocked validation honestly. Runtime changes require the explicit local Kubernetes topology; documentation alone does not validate runtime behavior.

## Why this approach

### Why not select services first?

A module owns behavior, a library shares a contract or mechanism, and a service introduces a deployment boundary. Their reasons differ. Extra processes cannot resolve unclear ownership.

### Why not fit existing code into the diagram?

The target model is written before the keep/replace mapping. Existing code supplies evidence and reusable guarantees; it does not dictate the target's vocabulary or topology.

### Does an event backbone make the system event-sourced?

The selected baseline uses PostgreSQL for authoritative transactional state and JetStream for durable delivery. Define retention and reconstruction explicitly; an event backbone does not require event-sourcing every aggregate.

## Where the detail lives

- [Design worksheets](project-workflow-design-worksheets.md): required event, API, component, recovery, and experiment outputs.
- [Architecture](../architecture/architecture.md) and [code ownership](../architecture/code-organization.md): current implementation references to verify.
- [Event platform](../architecture/event-platform.md): existing reporting and accepted-fact contracts.
- [Workflow evaluations](../reference/workflow-evaluations.md): current fixtures and the limits of their evidence.
- [Documentation rules](../contributing/documentation.md): source placement and validation.
