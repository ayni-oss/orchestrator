# MVP: a development request to a reviewable change

Status: agreed product direction from the private MVP discussion. This document
defines intended scope, not implemented capabilities. Implementation choices
called out below remain open. There is no public launch or delivery commitment.

## Product promise

A small team connects a repository, submits a bounded development request, and
receives a reviewable draft PR with reproducible Ayni evidence, using execution
infrastructure the team controls.

The product hypothesis is that prepared execution and trustworthy feedback reduce
the human effort needed to get an agent's change ready for review. The
[MVP validation plan](mvp-validation.md) defines how to evaluate that hypothesis.

The [delivery milestones](milestones.md) sequence implementation into one PR per
milestone. [Docker Desktop Kubernetes with
Kustomize](decisions/0001-local-kubernetes-deployment.md) is the selected local
deployment approach, beginning with the first running service.

## First audience and task shape

The initial pilot is one trusted team with a repository it actively maintains
and access to self-hosted execution infrastructure. Start with GitHub, one
repository per task, one documented Ayni project shape, and one agent integration.
The repository already has a usable Ayni policy and environment lock.

Use bounded maintenance work with acceptance criteria a maintainer can judge:
bug fixes with regression tests, small behavior changes, and contained refactors.
For example: "Fix the parser's handling of an empty value and add a regression
test." Record a starting revision and the expected result before execution.

Our orchestrator and worker application code should be Rust. The agent integration
and repository toolchains can use their own implementations. Self-hosted
execution does not imply an offline model or a locally hosted agent service;
record the selected integration's actual data and execution boundaries.

## User journey

### 1. Connect and prepare

The user grants access to a selected repository. The orchestrator validates access,
the supported project shape, configuration, and locked inputs, then prepares an
environment for an exact revision. Expose three understandable readiness outcomes:

- **Ready:** validated preparation is available for the recorded inputs.
- **Setup required:** the user receives a concrete configuration or access issue.
- **Preparation failed:** the user can inspect the failing step and diagnostics.

Missing configuration must not silently create or commit a policy. Retain and
reuse compatible preparation for subsequent tasks, and invalidate readiness when
relevant inputs change. An older successful build cannot mark newer inputs ready.

### 2. Submit a request

A small task page accepts a request, identifies its repository and starting
revision, and presents progress and results. Submission explicitly authorizes
the bounded development task. Each attempt receives its own checkout, branch,
environment identity, and enforced execution limits.

The page must expose current status, useful diagnostics, cancellation, and a path
to retry or provide requested input. A full conversational workspace is deferred.

### 3. Develop with one agent

The agent reads repository guidance, inspects and edits files, runs commands,
and uses focused Ayni verification to repair findings. Keep the integration
replaceable, but support only one tested agent initially.

Requests that need clarification or exceed the supported scope have an explicit
blocked outcome. The agent cannot silently weaken required quality policy or
grant itself capabilities to finish a task.

Dependency changes require either a tested path to refresh declarations and
locks, rebuild, and validate the candidate, or an explicit unsupported outcome.
Before the pilot, record which behavior the initial implementation supports.
Passing against stale preparation is never an acceptable fallback. The first
pilot can concentrate on changes within an unchanged environment.

### 4. Validate and return a draft PR

Freeze the candidate inputs and run an independent, unscoped managed Ayni check.
Collect final evidence through a controlled runner outside agent-writable storage.
The draft PR and task result identify:

- What changed and why, plus the exact candidate commit.
- The final validation result and relevant findings.
- References to retained source/environment identities and quality artifacts.
- Unresolved limitations, policy/test changes, and anything left unverified.

A task can retain partial work after failure, but must not present it as a verified
result. Creating a PR is not proof that validation passed. If the candidate changes
after validation, previous evidence describes only the earlier candidate.
The human reviews and decides whether to merge.

### 5. Recover and clean up

Keep the request, attempt history, useful checkpoints, and final evidence outside
disposable workers. After failure, retry safely or report interruption with a
clear next action. Resuming the exact agent conversation is not a prerequisite;
preserving useful work and reporting the actual outcome is.

Cancellation and execution limits stop owned work. Repeated events and uncertain
provider responses must not create duplicate PRs. Cleanup removes attempt-owned
resources while preserving evidence according to the configured retention policy.

## Scope boundary

| Included in the MVP | Deferred |
| --- | --- |
| One trusted team and controlled execution infrastructure | Multiple tenants and arbitrary untrusted contributions |
| GitHub and one repository per task | Other providers and coordinated multi-repository changes |
| Existing Ayni configuration and one supported project shape | Automated policy onboarding and broad language coverage |
| Validated preparation and compatible reuse | Advanced cache distribution and prewarming strategies |
| One tested agent integration | Agent marketplace, agent teams, and a new native coding runtime |
| Manual requests, task status, results, cancellation, and retry | Slack, schedules, autonomous task discovery, and full chat UI |
| Managed final validation and draft PR output | Automatic merge and production deployment |
| Documented setup, execution limits, and failure recovery | Development-service discovery and a service catalog |

## Relationship to implementation

The [first iteration investigation](first-iteration.md) describes the execution
foundation: pinned revision to managed evidence. Completing that slice alone
does not complete this product MVP; the user-facing request-to-PR journey must
also work.

The repository-structure discussion proposed one private Cargo workspace:

| Crate | Responsibility |
| --- | --- |
| `domain` | Task/attempt rules, identities, evidence requirements, and shared worker messages |
| `control-plane` | Persistent task service, API and small UI, provider integration, Kubernetes reconciliation, and evidence retention |
| `worker` | Bounded checkout preparation, agent execution, Ayni invocation, and controlled validation operations |

Both application crates depend on `domain`. They communicate through an explicit
worker protocol rather than importing each other's implementations. Shared worker
code does not collapse the separate permissions and executions for development
and final validation.

This is an implementation proposal. The first agent, worker topology/transport,
database, artifact storage, Kubernetes resource schema, access mechanism, and
exact supported project shape still need selection and validation. The proposed
Rust guidance is [rust-skills](https://github.com/leonardomso/rust-skills/blob/master/SKILL.md);
pin the complete skill and its rules when adding it to the repository.

## Completion and privacy

The MVP is ready for a pilot when the full journey works and the engineering gates
in the [validation plan](mvp-validation.md#engineering-gates) pass on the selected
deployment topology. Product value remains unproven until the pilot is assessed.

All orchestrator plans, implementation, pilot results, and examples stay private.
Any Ayni improvements must have a standalone Ayni justification and follow the
[private development boundary](../AGENTS.md#private-development-boundary).
The pilot does not authorize public publication or external customer onboarding.
