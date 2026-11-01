# The alpha workflow

This page describes the planned alpha experience. Current local setup and runtime
features are described in [the contributor preview](index.md) and
[Connect a repository](repositories.md). The complete workflow below is not yet
available in the interface.

## Choose the scope of work

A workspace brings together available repositories, instructions, context, skills
and MCP tools. A project tackles a chunk of changes and selects only the workspace
repositories it needs. Several workspaces can reuse a connected repository without
sharing their private project conversations.

Use project chat to explain the goal, research the selected repositories, develop
a plan and review its acceptance criteria. Chat remains the place to follow
progress, answer questions and discuss the results.

## Review the plan

Tasks are modular pieces of work. They can run independently, in order or as a
graph where a task waits for several other tasks. Independent ready tasks can run
in parallel within the approved allowance.

The plan chooses whether tasks share a PR, produce separate PRs or use linked
stacked PRs. A PR belongs to one repository; projects spanning repositories can
produce several linked PRs. You can follow each task to its commits, checks and
PR, including tasks delivered together.

Approval covers the exact plan and its selected repositories, permissions and
allowance. Execution and PR publication need explicit grants, which can be reviewed
with the plan. A new goal or expanded scope needs amended review.

## Let agents finish the work

Within the approved scope, agents inspect and change code, add or remove files,
update packages, write tests, build, run checks and repair failures. They continue
ordinary implementation and repair without asking you to approve each step.
They ask when an answer, more allowance or a scope/permission change is needed.

Ayni is the first supported environment and quality path for alpha. For a
repository with Ayni configured, Orchestrator reuses a verified runtime or builds
and publishes one when authorized. Dependency changes can require a newly prepared
environment before managed validation. Custom containers remain future work.

## Review the result

A task needs passing Ayni quality and evidence that it meets its acceptance
criteria. Coding completion, quality and PR publication are separate results; a
failed check remains visible with saved work and the next action.

Delivery ends at reviewable PRs. You retain merge authority. Results and unresolved
questions return to the project so you can decide what happens next.

Future inputs may start a project prompt from a GitHub issue or link work to a
milestone. Chat is alpha's initial input surface. See the
[roadmap](https://github.com/ayni-oss/orchestrator/blob/alpha/ROADMAP.md) for delivery
order and qualification requirements.
