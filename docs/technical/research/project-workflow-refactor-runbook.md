# Project workflow refactor runbook

Date: 2026-10-19. Status: research procedure; target architecture is not yet selected.

## What this establishes

- Design the target from product scenarios, authority, and recovery requirements before mapping existing components into it.
- Treat project conversations as the working interface for planning, execution feedback, and confirmed recovery.
- Investigate distributed event producers and a shared JetStream backbone, including project-agent consumption of task activity.
- Derive module, library, service, storage, and API boundaries from the resulting contracts.
- Use the [design worksheets](project-workflow-design-worksheets.md) to produce reviewable evidence at each gate.

This runbook defines a method, not implementation approval or a delivery-status tracker. Record delivery work in linked GitHub issues and the organization Project. Record architectural decisions in focused decision documents after review.

## Confirmed product constraints

| Topic | Direction confirmed by the maintainer |
| --- | --- |
| Workspace | Defines reachable repositories and shared instructions, tools, MCP integrations, and context. |
| Project | Owns an enduring objective, multiple conversations, plans, and accumulated task feedback. |
| Plan and task | Plans organize changes; tasks execute through runtimes built with Ayni. Results feed subsequent planning. |
| Compatibility | Development state is disposable; breaking schema and API changes are acceptable. |
| Tenancy | Multiple tenancy is part of the broader system. Deployment tenancy and user management are outside this refactor's scope. |
| Recovery | A task reports its failure or question in its originating conversation, also visible in project-wide activity. |
| Authorization | The agent prepares a recovery proposal; the engineer must confirm it before recovery executes. |
| Continuation | Reuse retained work where safe. The distinction between resuming an attempt and creating a new attempt requires design. |

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
| B: System evidence | Trace representative commands, events, reads, state writes, and external effects through current code. Capture dependency and authority maps. | A | Each factual claim has a source or reproducible observation; unknowns remain explicit. |
| C: Domain and recovery | Define ownership, vocabulary, state machines, invariants, and recovery approval semantics. | A, B | Walk the failure-and-continuation scenario without ambiguous authority or state. |
| D: Contracts | Design event families, consumer behavior, gRPC commands and watches, and persistence boundaries. | C | Duplicate, late, missing, and replayed delivery have specified outcomes. |
| E: Components | Assign logical owners; compare modules, shared libraries, deployable services, and worker processes. | C, D | Every contract and side effect has one accountable owner; deployment splits have evidence. |
| F: Feasibility | Run bounded protocol, recovery, streaming, and scheduling experiments. | D, E | Evidence resolves the highest-risk assumptions or sends their decisions back for revision. |
| G: Implementation plan | Map target components to keep, replace, retire, or adapt; define cutover and removal. | F | Each slice has a visible outcome, dependency, acceptance tests, and rollback boundary. |
| H: Delivery | Implement reviewed slices and exercise them on Docker Desktop Kubernetes. | G | The complete feedback/recovery loop meets its acceptance criteria under failure. |

At each gate, record findings, alternatives, decision owner, unresolved questions, and the evidence required to proceed. The engineer prepares the analysis; the maintainer resolves product choices. Do not repeatedly seek confirmation for routine investigation already authorized.

## Anchor scenarios

Use these scenarios throughout modeling, experiments, and acceptance. Include a timeline of user-visible state, durable state, agent activity, and external effects for each.

1. Two conversations contribute to one project; the second understands results and decisions from the first without copying its transcript.
2. An approved plan starts tasks. Progress remains visible while the engineer drafts a message or navigates elsewhere.
3. A task cannot continue. Its originating conversation explains the cause, retained work, and the decision needed; project activity shows the same unresolved item.
4. The engineer answers. The agent proposes recovery, including changed inputs and budget. Confirmation resumes safely or creates an explicitly linked attempt.
5. Two browser tabs confirm the same recovery. Only one execution starts; both converge on the same receipt.
6. A worker or planner disappears, an event is duplicated, or a result arrives after cancellation. No stale writer advances accepted state.
7. Results change an assumption. The project records evidence and the agent proposes a plan revision without rewriting the previously approved version.
8. The browser reconnects and the planner wakes after idle expiry. Both catch up without losing accepted activity or replaying external actions.

## First implementation slice

Select the slice after the contract and feasibility gates. The preferred proving scenario is one task failure, one conversation exchange, one confirmed recovery, and one resulting project update.

Include durable attention state, retained-work identity, recovery proposal, approval receipt, scheduling, streamed feedback, and evidence acceptance. A simulated success-only chat cannot establish the contract.

Before implementation, agree numeric targets for acknowledgement latency, visible progress latency, reconnect recovery, queue fairness, and event-to-agent reaction. Measure user-visible updates separately from transport heartbeats. Do not invent targets from existing polling intervals.

## Cutover and completion

A clean replacement is available because compatibility is not required. Still inventory active work, published branches, pull requests, and retained artifacts before selecting an actual cutover procedure.

- Specify how dispatch stops and writers are fenced before switching authority.
- Define whether old executions drain or terminate, and what happens to their late results.
- Establish backups or explicit reset approval, plus the last safe rollback point before new external effects.
- Remove superseded transports, schemas, flags, controllers, and documentation after cutover; assign removal to delivery issues.
- Regenerate contracts and managed locks through repository tooling. Preserve Ayni's independent product boundary.

Completion requires passing scenario and failure tests, accurate architecture documents, dependency enforcement, and removal of obsolete paths. Report blocked validation honestly. Runtime changes require the explicit local Kubernetes topology; documentation alone does not validate runtime behavior.

## Why this approach

### Why not select services first?

A module owns behavior, a library shares a contract or mechanism, and a service introduces a deployment boundary. Their reasons differ. Extra processes cannot resolve unclear ownership.

### Why not fit existing code into the diagram?

The target model is written before the keep/replace mapping. Existing code supplies evidence and reusable guarantees; it does not dictate the target's vocabulary or topology.

### Does an event backbone make the system event-sourced?

That is a separate storage decision. Compare authoritative database state with durable event delivery against event-sourced aggregates. Document retention and reconstruction requirements before deciding.

## Where the detail lives

- [Design worksheets](project-workflow-design-worksheets.md): required event, API, component, recovery, and experiment outputs.
- [Architecture](../architecture/architecture.md) and [code ownership](../architecture/code-organization.md): current implementation references to verify.
- [Event platform](../architecture/event-platform.md): existing reporting and accepted-fact contracts.
- [Workflow evaluations](../reference/workflow-evaluations.md): current fixtures and the limits of their evidence.
- [Documentation rules](../contributing/documentation.md): source placement and validation.
