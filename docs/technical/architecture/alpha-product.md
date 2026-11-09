# Alpha product contract

Agreed direction: 2026-11-01. Planned alpha requirements; this document does not
claim they are implemented. [ROADMAP.md](../../../ROADMAP.md) owns delivery order;
individual PRs own implementation and qualification evidence.

## Product outcome

A maintainer connects Ayni-configured repositories, selects a workspace and starts
a project for a chunk of changes. Project chat researches the relevant source and
context, develops a reviewed plan, and coordinates autonomous tasks through tests,
Ayni quality, repair and reviewable pull requests. Questions and accepted outcomes
return to the project throughout its lifetime.

Delivery stops at reviewable PRs. Agents do not merge, deploy or declare a project
successful solely because its code-quality gate passed. The requested behavior
needs evidence against the reviewed acceptance criteria.

## Responsibilities

| Owner | Responsibility |
| --- | --- |
| Ayni | Language/tool integrations, policy evaluation, normalized quality findings, exact locks and portable development environments. |
| Orchestrator | Workspace/project context, planning, permissions, agent/tool execution, runtime publication, isolation, dependency scheduling, accepted evidence, PR delivery and feedback. |
| Maintainer | Goals, workspace capabilities, reviewed plan/allowances, scope amendments and final PR/merge review. |

Ayni is the first supported alpha runtime path. Maintain an explicit integration
support matrix for its documented project shapes, platforms and tool combinations,
including execution evidence and unsupported/blocked cases. Consume its public CLI/artifact
contracts and qualify the exact admitted release/profile combinations; do not
claim every resolvable lock or platform is qualified. Custom containers remain a
future extension with a separate contract. Ayni remains independently useful.

## Workspace, project and task scope

| Record | Meaning |
| --- | --- |
| Shared catalog repository | Stable source identity and integration reference, reusable without duplicating remote repositories or exposing credentials. |
| Workspace | Available repositories and shared instructions, context, skills, MCP/provider capabilities and resource defaults. |
| Project | A chunk of changes, its goal, agent/model configuration, project context, chat, plans, tasks and results. |
| Plan version | Reviewed task graph, acceptance criteria, repository/environment impact, PR layout and execution/publication grants. |
| Task | Modular approved work targeting one repository, with an objective, expected checks, dependencies and bounded authority. |
| Attempt | One execution with frozen source, runtime, upstream selections, configuration/context revisions and allowance. |
| PR delivery group | The plan's selection of tasks delivered through one PR in one repository; groups can have explicit base/dependency relationships. |

All workspace repositories are available to every project's conversation research.
Workspace settings manage the environment and connected repository availability;
projects own goals, model selection, instructions and agent budgets. Workspaces do
not own agents. Each request prepares fresh read context with immutable repository
snapshots and discovered skills. The conversation selects the repositories needed
for the requested work, and each task acquires only its reviewed source scope and
dependencies. Availability alone never authorizes writes or execution.

Multiple workspaces may reuse one catalog repository. Chats and task grants remain
project-scoped. Removing membership affects future discovery; existing admitted
attempts keep their frozen scope and provenance. Migration 0040 seeds workspace
membership from prior attachments; migration 0041 makes project availability
inherited and migrates model configuration to independent project heads.

Fresh source research, skill discovery and automated task source selection still
need implementation and qualification. The settings and membership APIs alone do
not establish that execution flow.

## Project chat, context and capabilities

Project chat supports research, plan generation and validation, progress,
questions, decisions and subsequent work. Retain original conversation records,
plan approvals, repository observations and accepted task results. Context
retrieval selects bounded relevant evidence with source identities; compaction
retains access to originals and cannot invent authority or silently erase decisions.

Agents can list/read/search selected repository snapshots and retrieve earlier
project decisions and outcomes. Project facts and explicitly shared contextual
resources may inform work; private reasoning/transcripts from another project do
not become ambient context. Existing isolated conversation records can support
the project surface without requiring one global shared agent session.

Skills are versioned instructions/resources; MCPs are tools governed by explicit
scope, schemas, credentials and budgets. Workspace capabilities bound project and
task selections. Freeze selected revisions and invocation provenance, show what
was used, and explain unavailable capabilities. Installing a skill does not grant
filesystem, network, repository or external-effect permissions.

## Reviewed plans and autonomous execution

Review binds an exact plan version and its task graph, selected repositories,
acceptance criteria, environment impact, PR strategy and allowances. Approval can
include explicit execution and publication grants for ready tasks. Those grants
remain separately enforced and retained; merely creating a plan grants neither.

Within that authority, agents inspect source, edit/add/delete files, update
packages and manifests, regenerate supported locks, write tests, run commands,
build and repair until the task is complete or a real blocker is reached.
Ordinary tool calls and quality-repair rounds do not require repeated approval.
Time, tokens, resources, external effects and retries remain bounded and visible.

Escalate questions, scope/permission changes, exhausted allowance and uncertain
external effects through chat. Preserve saved work and causal evidence for exact
confirmed continuation. A transport retry cannot repeat an uncertain paid call or
forge operation; an agent's claim of completion cannot accept its own evidence.

## Environment-changing work

The original attempt's source, policy, lock and runtime evidence remain immutable.
An authorized manifest/dependency change produces a retained candidate source and
lock using Ayni's public tooling. Trusted orchestration validates the candidate
and prepares a matching environment when required, then binds validation or a
linked continuation to its exact source/lock/runtime identities.

Do not validate a changed lock against an incompatible old image or report host
checks as managed evidence. Repository code and dependency scripts execute without
signer, registry-push or unrelated provider credentials. Signing, publication and
final verification retain independent ownership. Runtime re-preparation consumes
an explicit grant and allowance, which may be reviewed with the plan.

Policy changes need an explicit reviewed quality-policy requirement. Agents must
not lower thresholds, disable required checks or remove meaningful tests to hide a
failure. The candidate-environment/continuation protocol is implementation work;
current one-shot tasks still protect `.ayni.toml` and `.ayni.lock` from writes.

## Task graphs and PR structure

A single task, ordered sequence and DAG use the same task model. Validate all
edges and reject cycles, missing references and invalid repository selections
before approval. Schedule ready tasks within capacity; independent tasks may
continue when another branch of the graph is blocked.

The reviewed plan chooses one PR for several tasks, separate task PRs, stacked PRs
or a combination. Every PR belongs to one repository. Retain the mapping from
project/plan/task/attempt to delivered commits, PR groups, bases and related PRs,
so modular tasks remain individually inspectable even when delivered together.
A PR group requires managed Ayni quality and acceptance evidence for its exact
assembled source, including integration changes; passing component tasks alone
cannot establish the final PR result. Retain each task's contribution and evidence
when publication waits for other tasks in its group.

Changing approved scope or PR delivery relationships requires a new reviewed
version; it cannot rewrite already admitted work or publication receipts.

Downstream attempts consume explicit accepted upstream commits or artifacts, even
before human merge when the plan allows it. Freeze selections and integrated
source. Later upstream edits, retries and moving branches cannot silently replace
those inputs. Integration conflicts become authorized resolution work or an
amendment request; they cannot be hidden as a successful dependency handoff.

For same-repository dependencies, verify integrated Git ancestry. Cross-repository
edges instead name exact producer commits/artifacts or contract versions and the
consumer checks that prove compatibility. Neither a task-complete event nor Git
ancestry alone proves the whole project's requested behavior.

Current dependency execution requires retained published upstream commits and
manual source integration. Flexible PR grouping, server-side integration and
cross-repository dependency evidence are alpha requirements, not delivered behavior.

## Durable execution and observation

PostgreSQL retains ownership, approval, budgets and accepted outcomes. Extend the
existing outbox/JetStream delivery pattern for task dispatch, reports, authorization
decisions and cancellation. A broker receipt never confers execution authority.
Trusted communication/supervision owns messaging credentials outside the coding
process; large source/evidence payloads use immutable bounded artifact references.

Live text and tool activity may use bounded transient streaming. Accepted history
and terminal outcomes remain durable; reconnect reconstructs state without
replaying external effects. Periodic reconciliation repairs missed observations
and adopts only the exact owned workload. See [task event direction](task-events.md)
for the planned transition from current HTTP polling/callbacks.

## Completion and exclusions

A task is delivered when its intended changes and tests are retained, managed
Ayni quality passes with complete required evidence, acceptance criteria have
recorded results and its selected PR delivery is reviewable. Execution, quality,
acceptance, cleanup and publication have separate outcomes. Project chat explains
progress and outstanding work; humans retain project and merge review.

Alpha acceptance includes actual providers and repository/registry/forge operations,
standalone/sequential/DAG tasks, environment-changing work, skills/MCP use and the
current browser journey across interruption and restart. Deterministic fixtures
remain useful protocol tests but do not establish autonomous engineering ability.
Record interventions, admitted versions, workload limits and blocked checks.

External triggers (including GitHub issues and milestone-linked project prompts),
custom containers, multi-user tenancy, automatic merge/deploy and sustained
thousand-agent capacity qualification remain later scope. The internal event
architecture supports that direction without claiming those products exist.
