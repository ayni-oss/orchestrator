# Technical documentation

The runtime combines the M1 durable foundation with shared immutable agent configuration and isolated conversation sessions with scripted and opt-in OpenAI execution. PostgreSQL owns workflow and session authority; authenticated durable delivery and the trusted-loopback API remain v1 through alpha. Real provider calls retain request identity, uncertainty and nullable usage; bounded read tools retain frozen inputs and invocation evidence.

## Current implementation

- [Execution timeline](architecture/execution-timeline.md): retained accepted transitions, stable history pages and the workspace inspector.
- [Project workspace](architecture/project-workspace.md): Svelte project navigation, chat, retained activity, exact retries and scoped console handoff.

- [Agent plan proposals](architecture/agent-proposals.md): bounded terminal provider proposals, authenticated provenance and retained review.
- [Task dependencies](architecture/task-dependencies.md): reviewed DAGs, exact upstream commits, trusted source integration and four isolated execution slots.
- [Operational acceptance](operations/operational-acceptance.md): cumulative failure gates, concurrent capacity, cancellation and retained replay evidence.
- [Task admission](architecture/task-admission.md): exact-version operator approval, atomic receipts and tasks, immutable origin and awaiting-execution blockers.
- [Durable plan review](architecture/plan-review.md): operator-authored immutable versions, scoped review/history, exact retries and console editing.
- [Task feedback](architecture/task-feedback.md): immutable execution/quality notices in the originating conversation and project attention; no continuation authority.
- [Factual project continuity](architecture/project-continuity.md): bounded task lifecycle facts and accepted outcomes in frozen project-status inputs.

- [Architecture](architecture/architecture.md): authority, transactions, API surface, projections, and failure boundaries.
- [Provider requests](architecture/provider-requests.md): foreground OpenAI request lifecycle, reservation, cancellation and uncertain-outcome recovery.
- [Conversation execution](architecture/conversation-execution.md): consolidated M2 contract, alpha v1 policy, frozen configuration, isolated sessions, provider uncertainty and tool authority.
- [Code organization](architecture/code-organization.md): production ownership and dependency direction.
- [Local development](operations/local-development.md): Docker Desktop deployment, diagnostics, validation, restart, and cleanup.
- [Task delivery acceptance](operations/task-acceptance.md): cumulative M4 interruption, fencing, publication uncertainty and retained cleanup gates.
- [Plan acceptance](operations/plan-acceptance.md): cumulative M3 approval, proposal, outage and retained-origin recovery gates.
- [Conversation acceptance](operations/conversation-acceptance.md): cumulative M2 recovery gates and evidence limits.
- [Operations](operations/operations.md): startup, probes, schema changes, credentials, backup, and failure handling.
- [Protocol tooling](../../proto/README.md): canonical protobuf generation and compatibility policy.

## Foundation contracts and research

- [Plans and approval](architecture/plans-and-approval.md): M3 implementation contract for immutable versions, explicit approval, task admission and retained project/conversation origin; review, agent proposals and approval/admission are implemented; cumulative M3 gates are defined with deployed evidence required in the final delivery PR.
- [M3 delivery plan](research/project-workflow-m3-delivery-plan.md): four cumulative PRs, contract completion gates and failure/recovery acceptance.
- [Task recovery review](architecture/task-recovery.md): immutable proposals, saved evidence and operator question/answer records without execution authority.
- [M6 delivery plan](research/project-workflow-m6-delivery-plan.md): five slices from durable attention to confirmed continuation, with preserved work and explicit allowances.
- [Scoped MCP integrations](architecture/mcp-integrations.md): frozen operator grants, exact external-effect consent, bounded tools and uncertainty.
- [M7 delivery plan](research/project-workflow-m7-delivery-plan.md): factual continuity, originating-conversation assessment, reviewed proposals, scoped MCP integrations, additional providers and cumulative acceptance.
- [M1 durable foundation](research/project-workflow-m1-durable-foundation.md): full milestone contract and later-slice handoffs.
- [M1 acceptance evidence](research/project-workflow-m1-acceptance-evidence.md): executable gates, reproduction, and coverage limits.
- [Feasibility evidence](research/project-workflow-m1-feasibility-evidence.md): closed pre-implementation evidence.
- [Refactor runbook](research/project-workflow-refactor-runbook.md) and [design worksheets](research/project-workflow-design-worksheets.md): investigation provenance.

The [event-platform history](history/event-platform/index.md) is retained as dated evidence, not as current runtime documentation. Superseded browser, SQLite workflow, runtime-worker, provider, compatibility-drain, and custom-client instructions were removed during the clean cutover.

[Documentation authoring](contributing/documentation.md) defines the audience boundary. The [roadmap](../../ROADMAP.md) tracks active product refinement. The
[backend foundation roadmap archive](history/backend-foundation-roadmap.md) retains
M1–M8 delivery scope and acceptance limits.

- [Contributor console](architecture/contributor-console.md): local browser transport, discovery, and retry boundaries.

- [Task execution preparation](architecture/task-execution.md): trusted operator evidence and frozen attempts.
- [M4 delivery plan](research/project-workflow-m4-delivery-plan.md): six staged execution, quality and publication scopes.

- [Isolated task execution](architecture/isolated-task-execution.md): durable bounded concurrent dispatch, bounded coding and cancellation fencing.

- [Task quality](architecture/task-quality.md): frozen validation, managed Ayni execution and separate retained evidence.

- [Task draft publication](architecture/task-publication.md): explicit scoped authority, deterministic Git changes and recoverable draft identity.

- [Cumulative recovery acceptance](operations/recovery-acceptance.md): continued-attempt fault matrix, replay, retained cleanup and unmet M6 prerequisites.
