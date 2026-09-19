# Product intent

Status: exploratory. Updated 2026-09-15.

This document preserves the broader product vision. The [MVP](mvp.md) narrows the
first delivery to one repository, one agent integration, and the request-to-draft-PR
journey. Use its [validation plan](mvp-validation.md) for the initial pilot.

## Origin and problem

Ayni began as an alternative to a SonarCloud-style quality workflow, designed
to give coding agents a short, actionable feedback loop. Its environment work
connects repository policy to the runtimes and tools needed to measure it. Using
that loop with agents suggests a broader opportunity: automate the preparation,
execution, and verification surrounding development work.

The working hypothesis is that teams need less repeated setup and more consistent
acceptance evidence as agents produce more changes. A prepared machine alone
does not define acceptable work. A quality report alone does not provision a
workspace, start its database, or resume a task after a worker disappears.

## Intended experience

An organization connects selected repositories through a provider integration.
The platform discovers their requirements, prepares reusable environments, and
exposes readiness and setup failures.

A user selects a project or sends a request through an approved external channel.
The task receives project context, authorized repository access, suitable
environments, and required development services. An agent discovers the work,
implements it, uses Ayni's feedback loop, and returns a reviewable change with
evidence. A conversation can continue while execution pauses or changes workers.

Projects may span multiple repositories. Agent count is independent of repository
count: one agent can work across several repositories, and additional agents can
be assigned when the task benefits from parallel work.

## Product boundaries

| Product or component | Intended responsibility |
| --- | --- |
| Ayni | Local environment and quality contracts, adapter-owned tool execution, normalized findings, and focused verification |
| Orchestrator | Project and task lifecycle, workspace preparation, supporting services, access policy, execution recovery, and evidence retention |
| Agent runtime | Understanding requests, discovering dependencies, planning, editing, choosing useful additional checks, and interpreting findings |
| Repository provider | Repository identity, access grants, commits, branches, PRs or merge requests, and provider events |

Cursor compatibility is a first-class intention. The platform should be valuable
when Cursor owns the conversation and agent loop. A built-in agent may become an
option, but is not a prerequisite for validating the environment and evidence
value proposition.

Keep provider orchestration and Kubernetes concerns in this project. Do not
silently move forge-specific APIs or lifecycle policy into Ayni's local,
forge-neutral core.

## Audience and adoption hypothesis

The initial audience hypothesis is platform engineering teams that already
operate Kubernetes and want repeatable agent development workflows on infrastructure
they control. A self-hosted installation should provide a complete useful workflow.

Kubernetes is the preferred direction to explore because projects may need several
workspaces, databases, queues, resource limits, and lifecycle management. Merely
using Kubernetes does not make adoption easy. Installation, provider onboarding,
diagnostics, upgrades, and cleanup are part of the product.

An eventual SaaS could offer managed operation, capacity, environment caching,
artifact retention, and organization administration. A hosted control service
with execution in customer clusters is another option. These are business
hypotheses; tenancy, licensing, pricing, and hosting commitments are open.

## Two connected flows

### Repository readiness

Repository enrollment and subsequent changes cause the platform to reconcile
prepared environment state. Users can see which repositories are ready, which
need a lock refresh, and which failed preparation. New tasks can reuse validated
artifacts without repeating expensive setup.

### Development requests

Chat, an API, or an authorized event creates or continues a task. The platform
starts the needed workspaces, routes agent access, retains conversation and
execution state, and returns findings and changes through appropriate channels.

An incoming event is not automatically authorization to edit code. For example,
a push may only refresh readiness, while an approved issue label can request work.

## Intended benefits

- Start useful verification without repeatedly rediscovering tool installation.
- Carry project knowledge across tasks without treating learned advice as policy.
- Make coordinated changes across repositories with different toolchains.
- Provision disposable development services and synthetic test data.
- Use the same repository quality contract locally, in CI, and remotely.
- Explain what passed, what failed, and what remains unmeasured.
- Recover from worker failures without losing the task or duplicating external work.

## Claims to avoid

- Universal repository support or "all required tools." Begin with Ayni's
  documented managed project shapes and explicit extension points.
- Environment reproducibility as a guarantee of deterministic results. Flaky
  tests, external systems, runtime conditions, and test data can change outcomes.
- Green signals as proof of feature correctness. Product acceptance and
  integration behavior may need additional evidence.
- Multi-repo and webhooks as unique capabilities. Cursor documents both; see the
  [comparison](integrations-and-positioning.md#competitive-baseline).
- An implemented orchestrator. These documents preserve intent, not availability.

## Early boundaries

Explore request-to-reviewable-change first. Autonomous production deployment,
unbounded recursive agent spawning, a universal workflow language, and complete
feature parity with mature coding products are outside the initial experiment.
This is a scope proposal, not a permanent exclusion.

There is no public launch commitment. See the [publication boundary](../README.md#publication-boundary).
