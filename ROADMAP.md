# Roadmap

This roadmap records agreed product direction. Requirements and delivery status live in the
[Orchestrator Project](https://github.com/orgs/ayni-oss/projects/2) and
[repository issues](https://github.com/ayni-oss/orchestrator/issues).
Use those records to track milestones and acceptance criteria.

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

These slices describe the agreed direction. Use their linked delivery issues and
the Orchestrator Project for current status and acceptance criteria.

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
