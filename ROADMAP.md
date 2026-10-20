# Roadmap

This roadmap is the source of truth for agreed product direction and milestone scope.
Each pull request is its own work item and records requirements, acceptance criteria,
validation evidence, and delivery status, linking back to the relevant section here.
Do not create separate GitHub issues for roadmap work. Optional GitHub milestones
can group related PRs; reuse a matching milestone or create one when useful.
Update this roadmap in the same PR when direction or milestone status changes.
Development branches start from the latest `origin/alpha`; PRs target `alpha` explicitly.

## Project workflow replacement

Direction agreed 2026-10-19. This is the proposed eight-milestone delivery sequence,
not a claim that the replacement is implemented or a calendar estimate. Detailed
requirements and execution evidence belong in the PRs delivering each milestone.
Define each PR's acceptance criteria before implementation.

Build on fresh branches from `alpha` in this repository and merge PRs into `alpha`.
Development state is disposable and breaking changes are required where the old
design conflicts. Prior SQLite schemas, retained data, APIs, tests, and broker topology
are research input only: do not migrate, dual-write, preserve, or adapt them. M1
runtime slice 1 deletes the superseded workflow implementation and tests while
installing the replacement boundary; every later milestone builds only on the
replacement contracts and cumulative acceptance harness. Repository separation
remains deferred. PostgreSQL is the target transactional store, JetStream carries
durable messages, and gRPC provides command and streaming contracts. Service topology
follows ownership and failure contracts.

The [refactor runbook](docs/technical/research/project-workflow-refactor-runbook.md)
defines research gates; its [worksheets](docs/technical/research/project-workflow-design-worksheets.md)
define required design evidence. The [M1 durable foundation contract](docs/technical/research/project-workflow-m1-durable-foundation.md)
applies them to the first executable slice. Research gates A–H are distinct from
delivery milestones M1–M8 below. Apply the relevant gates before each implementation;
feasibility experiments may revise the decomposition.

### Product constraints

- Conversations share agent configuration but retain isolated working context.
  A fresh conversation receives the objective and factual project status, excluding
  other conversations' transcripts, plans, and reasoning. Cross-conversation memory is deferred.
- Plans are developed in their originating conversations and visible at project level.
  Concurrent work uses isolated worktrees. Tasks target one repository initially.
- A dependent task can use an explicitly selected upstream commit before merge.
  Its admitted inputs stay frozen when the upstream branch changes.
- Questions and failures reach the originating conversation and project activity.
  Recovery requires a proposal and confirmation; retained work has an explicit retention policy.
- Task delivery ends at draft PR creation. The user decides project completion,
  supported by an agent summary. Automatic assessments may propose further work without authorizing execution.
- Capability priority is workspace instructions, then MCP integrations, then multiple providers.
  Tenancy expansion and user management are outside this refactor.

### Delivery milestones

| Milestone | Deliverable | Demonstration required to finish |
| --- | --- | --- |
| M1: Durable foundation | PostgreSQL persistence and migrations, JetStream publication/consumption, gRPC commands and watches, minimal workspace/project/conversation records. | A durable message is processed by a scripted executor and streamed back despite restarts, duplicate delivery, and reconnects. |
| M2: Conversation execution | Shared agent configuration, isolated sessions, workspace instructions, one real provider adapter, and structured activity. | Two conversations stay isolated; preparation/model activity is visible; reconnect restores accepted state. |
| M3: Plans and approval | Conversation-owned plans, immutable versions, approval receipts, and task admission. | Approval admits exactly the reviewed task set once; stale or conflicting approval is explained. |
| M4: Isolated task execution | Ayni runtimes, worker supervision, single-repository tasks, retained evidence, and draft publication. | A real task creates a draft PR with separate execution, quality, and publication outcomes. |
| M5: Dependencies and concurrency | Concurrent worktrees, dependency scheduling, and explicit upstream commit selection. | Task B uses A's selected commit before merge; later changes to A cannot silently change B's inputs. |
| M6: Feedback and recovery | Attention items, waiting state, retained work, recovery proposals, and confirmation. | A task asks for help and continues safely after confirmation, including across pod loss. |
| M7: Project continuity and integrations | Factual summaries, automatic outcome assessment, proactive proposals, MCPs, and additional providers. | Fresh conversations receive only objective and factual status; feedback reaches its origin and project activity; integrations obey configured scope. |
| M8: Operational acceptance | Retention, diagnostics, capacity checks, self-development scenario, cutover, and obsolete-path removal. | Failure scenarios pass and Orchestrator can develop Orchestrator without relying on it as the only repair mechanism. |

Each milestone builds on the preceding executable slice and retains its regression
suite. M1–M6 establish the core planning, execution, and confirmed-recovery loop.
M7–M8 complete the agreed initial scope and operational acceptance. These labels
refer to this replacement sequence, not historical GitHub milestone numbers.

### M1 executable slice

Specify the contracts, logical module map, local stack, and executable acceptance
cases before implementation. Complete the contract's bounded PostgreSQL, JetStream,
executor-handoff, watch, and protobuf feasibility experiments before a runtime PR.
The replacement acceptance harness is the only delivery baseline; historical sandbox
outcomes do not constrain its states or architecture. Runtime slice 1 deletes the old
workflow implementation and tests instead of depending on or preserving them. M1
deliberately resets the private v1 workflow API and updates all known clients with the server; it exposes
that API only on the trusted loopback operator listener. M1 establishes Workspace
as the required parent identity for Projects and Conversations; shared workspace
instructions and agent configuration remain M2 scope.

1. Submit one or more conversation messages through gRPC; active-work messages wait
   in a bounded FIFO without steering the current Turn.
2. Save each message, Turn, dispatch intent, receipt, and eligible command in one
   PostgreSQL transaction.
3. Publish retained exact command bytes to authenticated JetStream without treating
   broker storage as executor ownership.
4. Let a scripted executor retain the command, win an application-decided ownership
   claim, and only then produce a retained response.
5. Validate and persist each report and its immutable decision receipt through the
   application owner; keep both sides' retry evidence until the next owner confirms it.
6. Deliver accepted updates through a resumable gRPC watch.
7. Disconnect and reconnect using the accepted cursor.

A minimal diagnostic client is sufficient for this first proof. Scripted execution
makes failure cases reproducible; M2 adds the real provider and working conversation
interface. Logical owners include admission, persistence, event delivery, executor
protocol, and activity projection; they need not be separate services.

M1 acceptance must establish:

- Repeating a command returns its original receipt; duplicate delivery produces one accepted response.
- A broker-stored command survives application restart, and PostgreSQL republishes
  one that expires before an executor durably claims it.
- A crash between database commit, publication, ownership claim, report acceptance,
  and either decision receipt loses no accepted work or exact retry bytes.
- Concurrent FIFO admission, queue limits, failed predecessors, context freezing,
  promotion after restart, and one-at-a-time eligibility behave deterministically.
- A watcher reconnects without gaps; expired cursors have an explicit snapshot recovery path.
- Broker unavailability or stream/ledger capacity leaves work visibly pending and recoverable.
- Cancelling a watch does not cancel its underlying operation.
- Accepted, queued, processing, completed, and failed states are distinguishable.
- Anonymous/cross-role broker access, unsupported-version loss, and split-brain
  executor progress are denied by executable tests.

The M1 contract proposes numeric targets for durable acknowledgement, visible
activity, and completion. A maintainer must approve or revise them before the
acceptance suite becomes a delivery gate. Transport heartbeats do not prove
execution progress.

### Validation at every milestone

| Level | Purpose | Evidence |
| --- | --- | --- |
| Contract | Verify policy independently of infrastructure. | Valid transitions, scope, frozen inputs, approval binding. |
| Integration | Exercise actual storage and transport. | PostgreSQL transactions/migrations, JetStream redelivery, gRPC replay. |
| Failure | Interrupt specific durability boundaries. | Crashes before publication, after processing, and before acknowledgement. |
| User scenario | Verify what the engineer observes and can do. | Send, watch progress, reconnect, answer, confirm, and continue. |

Keep one cumulative acceptance harness. Add real Docker Desktop Kubernetes tests
when workloads enter the system and provider-backed tests when model behavior matters.
Passing scripted tests does not prove worker isolation, checkpoint safety, quality,
or publication behavior. Every delivery PR must name prerequisites, observable
outcomes, failure cases, acceptance commands, and obsolete paths to remove.

## Existing implementation and earlier delivery direction

The sections below describe the existing SQLite-based implementation and its earlier
delivery sequence as learning context only. They confer no compatibility, migration,
data-preservation, testing, API, or topology requirement on the replacement. The
replacement direction above governs the new backend; historical issue links retain
their original meaning.

## Workspaces, projects and agent execution

Orchestrator provides reusable workspaces where users pursue specific projects.
A read-only planner researches each project and develops plans for human review.
Workers execute approved tasks and return evidence to the originating conversation.

The product model is **Workspace → Project → Plan → Task → Run**.

| Name | Responsibility |
| --- | --- |
| Workspace | Shared repositories, runtimes, connections, tools, skills, policies and agent defaults. |
| Project | A specific outcome, its selected repositories and the scope of permitted changes. |
| Conversation | Durable project discussion, decisions, questions and research history. |
| Plan | A versioned proposal containing findings, scope, acceptance criteria and proposed tasks. |
| Task | Approved, bounded work tied to a specific plan version. |
| Run | One task execution attempt. A retry creates another run. |
| Planner | The agent that researches, clarifies requirements and develops plans within a project. |
| Worker | The agent that edits code and executes an approved task. |
| Session | A replaceable live planner instance associated with a conversation. |
| Agent Runtime | The software that runs the model loop and routes tool calls. |

Workspaces currently own repository membership and shared agent defaults.
Outcome-focused Projects select workspace repositories while retaining conversations
and task history. Conversations retain versioned plans, and approval binds the exact
reviewed version to immutable task inputs. Multiple Projects can share a Workspace.

### Product interface

The following interface and idle-timeout controls describe the planned working surface.

The Project page is the main working surface, with **Overview, Plans and Tasks** views.
Overview places a fixed-height planner chat above the current plan card.
Message history scrolls within the chat, leaving the plan card stationary as the conversation grows.
Plans retains proposal versions and approval history; Tasks exposes worker progress, individual runs and results.

The Workspace page lists Projects and provides access to shared configuration.
Workspace defaults include the planner profile and idle timeout.
Projects select repositories and planning instructions; conversations can override their idle timeout.

### Planning and approval

The Planner has read-only access to repositories and documentation.
It records research, asks for missing requirements and prepares actionable plans.
The Control Plane persists state, validates approvals, enforces permissions and budgets, and manages execution resources.
Workers perform repository edits and execute checks within their approved task scope.

Approval binds a plan version to its executable tasks.
Further discussion produces a new plan version without changing tasks already admitted.
A worker receives the objective, acceptance criteria, research findings, source revisions, constraints and verification requirements.
Its results return to the originating project conversation with changes, evidence, unresolved findings and questions.

Execution, quality and publication retain independent outcomes.
The Planner interprets worker results so users can continue the discussion and decide subsequent work.

### Durable context and reusable sessions

Conversation records persist in SQLite on the Control Plane's persistent volume,
independently of planner environments. The archive retains messages, plan decisions,
research references and worker outcomes; conversation messages are not JetStream events.
Each run freezes
a versioned working index, recent messages and relevant older records for bounded retrieval.

Reusable sessions keep one live planner environment per conversation and process turns serially.
Workspace idle timeouts default to five minutes and can be set to 24 hours.
Conversations inherit that setting or override it with either preset.
Accepted user messages reset the idle timeout; an expired session is removed once it has no active turn.
A later message recreates the session and restores its context without losing history.

Session expiry does not interrupt an active turn.
Per-turn deadlines and model-call budgets remain separate from the idle timeout.
Repository observations and credentials are refreshed between turns. Changed model
credential bindings replace the session generation before the next turn.

The working index contains extractive excerpts with original record IDs and a
source watermark. Decisions, questions and references remain in their original
messages and research records. Search retrieves older records by relevance to the
current question; a partial frozen selection explicitly reports that more history exists.
Compaction keeps the original archive and cannot overwrite a newer index.

### Implementation sequence

These slices record earlier delivery direction. Their linked issues preserve
historical context; new work follows the roadmap and PR workflow above.

| Slice | Observable outcome | Issue |
| --- | --- | --- |
| Workspace and Project model | Shared configuration belongs to Workspaces; outcome-focused Projects retain their conversation and task links. | [#183](https://github.com/ayni-oss/orchestrator/issues/183) |
| Versioned plans and approval | Approval binds reviewed plan versions to immutable task inputs. | [#184](https://github.com/ayni-oss/orchestrator/issues/184) |
| Durable conversation context | A recreated planner can recover prior decisions and retrieve older research without relying on pod memory. | [#185](https://github.com/ayni-oss/orchestrator/issues/185) |
| Reusable planner sessions | Consecutive messages reuse a session; configured inactivity expiry preserves history and allows recreation. | [#186](https://github.com/ayni-oss/orchestrator/issues/186) |
| Project working interface | Overview combines fixed-height chat and the current plan; Plans and Tasks provide review and execution history. | [#187](https://github.com/ayni-oss/orchestrator/issues/187) |
| Worker feedback | Progress, questions and evidence return to the originating conversation with independent execution, quality and publication outcomes. | [#188](https://github.com/ayni-oss/orchestrator/issues/188) |
| Agent capability foundations | Agent profiles define models, instructions, pinned skills, allowed tools, resource scope and budgets. | [#189](https://github.com/ayni-oss/orchestrator/issues/189) |
| Workflow evaluation | Repeatable scenarios verify research, clarification, approval, execution, recovery and result interpretation across the complete flow. | [#190](https://github.com/ayni-oss/orchestrator/issues/190) |

Capability foundations provide attachment points for future Model Context Protocol tools and skills.
Application code enforces permissions on tool calls and records their results; skills do not grant additional authority.
Broad integration support follows a working planning and execution flow.

Workflow traces expose research activity, clarification, proposal preparation, approval waits and worker outcomes.
Evaluations cover planning quality, read-only boundaries, session recreation, duplicate prevention and result interpretation.
Tool and skill evaluations also cover activation boundaries and successful execution.

### Open choices and delivery boundary

Moving conversation history to JetStream and adopting a provider-specific agent framework have not been decided.
Custom Runtime and coordinated multi-repository task dependencies remain separate work.

The current checkout includes the repository image ownership correction:
runtime images use `ghcr.io/<repository-owner>/<repository>:ayni-<arch>-<lock>`.
Authentication usernames are excluded from browser responses, and dialogs display the full image destination.
Connection-test namespaces remain separate from runtime image ownership.
See [managed runtime execution](docs/technical/guides/managed-runtime-execution.md) for the image naming contract.

## Existing research and environment work

The [M7 — Workspaces Projects milestone](https://github.com/ayni-oss/orchestrator/milestone/8)
tracks the product direction above and retains the existing open work:

| Work | Source of truth |
| --- | --- |
| Later research: disposable execution, stage separation and independent assessment | [#128](https://github.com/ayni-oss/orchestrator/issues/128) |
| Approved dependency/lock changes with local checks and post-merge runtime preparation | [#129](https://github.com/ayni-oss/orchestrator/issues/129) |

Plans assess dependency and environment impact before approval.
Authorized workers regenerate `.ayni.lock`, install dependencies locally, and run applicable checks before opening a PR.
They do not build or publish a proposed runtime image.
After owner review and merge, repository sync reuses a compatible verified image or builds and verifies a new shared runtime.
The execution runtime and proposed lock remain separate evidence identities; quality thresholds and trusted keys require separate authorization.

The [product flow](docs/technical/guides/ayni-product-flow.md) and
[architecture](docs/technical/architecture/architecture.md) describe current behavior.
[Reviewed environment changes](docs/technical/guides/environment-changes.md) records accepted
direction that is not yet implemented. Custom Runtime and coordinated
multi-repository tasks also remain unsupported.

Completed work remains in closed issues, merged pull requests and
[closed milestones](https://github.com/ayni-oss/orchestrator/milestones?state=closed).
[Decision records](docs/technical/decisions/README.md) and
[dated event-platform research](docs/technical/history/event-platform/index.md) preserve
design history without becoming a second delivery plan.

Ayni's independent CLI roadmap belongs in its own repository.
