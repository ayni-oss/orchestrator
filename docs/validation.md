# Open questions and validation

Status: broader exploration backlog, not an implementation roadmap or release
commitment. The [MVP scope](mvp.md) and [MVP validation plan](mvp-validation.md)
define the current pilot. Experiments below, including multi-repository work and
development services, do not expand that scope.

## Decision status

### Recorded intentions

- Keep these plans in this private repository until an explicit
  publication decision.
- Keep Ayni useful independently as a local environment and quality product.
- Explore an eventual open-source orchestrator with an optional SaaS path.
- Explore Kubernetes for repository environments and development services.
- Support persistent projects spanning multiple repositories.
- Let agents discover affected repos, services, and useful checks; automatically
  record the actual execution inputs rather than require hand-authored combinations.
- Treat Cursor compatibility as a first-class product direction.
- Preserve configurable skills, MCP connections, tools, and agent implementations.

### Working hypotheses

- Platform teams with existing Kubernetes infrastructure are the first audience.
- An independent final validation path improves trust in agent-generated changes.
- Prebuilt environments reduce setup failures and time to useful verification.
- Separate repo environments can preserve toolchain independence in multi-repo work.
- The platform can be valuable even when an external product owns the agent loop.

### Unselected implementation choices

| Question | Alternatives or considerations |
| --- | --- |
| First agent | Cursor integration, another existing agent, or thin Rig runtime |
| Workspace topology | Combined VM/container, multi-container pod, or separate repo pods |
| Command transport | Kubernetes execution gateway, in-environment executor, or agent-specific bridge |
| Durable state | Application storage and Kubernetes lifecycle resources; schema and database unchosen |
| Image and preparation transport | Private registry, cache strategy, prepared-output reconstruction |
| Service declarations | Existing repository manifests, agent-generated proposals, or a small project contract |
| Acceptance policy | Repository baseline, project integration requirements, approval of policy changes |
| Isolation and tenancy | Trusted teams, untrusted contributions, tenant boundaries, runtime choices |
| Public distribution | License, packaging, support expectations, and publication timing |
| SaaS | Fully hosted service versus hosted control with customer execution |

## Proposed experiments

### 1. Ayni with Cursor

Goal: establish useful integration without building a replacement coding agent.

Use a supported repository and a real behavior change. Exercise focused
verification, repair a finding using its exact command, and complete an unscoped
managed check. Confirm that source and environment identities match the intended
candidate. Record any runtime or agent-tool limitations.

Exit evidence: working end-to-end trace, full Ayni artifact, environment identity,
and a clear account of manual setup. If managed execution is blocked, record the
blocker; host-only results do not establish managed compatibility.

### 2. Candidate environment evolution

Goal: prove that a dependency change is reproducible before merge.

Start from a ready environment, let the agent add a dependency, refresh required
declarations and locks, then rebuild and validate from clean inputs. Remove
unrecorded live installations before the final attempt. Verify that the merged
inputs either match the candidate or trigger a new build.

Exit evidence: baseline and candidate identities, a fresh successful run, and an
intentional failed-build case that does not masquerade as current readiness.

### 3. Multi-repo change with a development service

Goal: prove coordinated work and automatic recording without making the user
predeclare every relationship.

Use two repositories, such as an API and its frontend, plus a disposable database.
Have the agent discover the required changes and checks. Run repo contracts and
an integration check against candidate inputs. Confirm the record reflects what
ran, including the fixture revision and any local dependency substitution.

Exit evidence: coordinated proposed changes, per-repo results, integration output,
captured execution identities, and an explanation of merge ordering.

### 4. Lifecycle recovery

Goal: verify that task state survives execution failure without duplicate effects.

Repeat a webhook, restart a controller, interrupt a worker, and simulate an
uncertain provider response after a PR creation. Check that the task resumes or
reports interruption clearly, avoids duplicate PRs/notifications, and cleans up
resources without deleting active work.

Exit evidence: task history, deduplicated external actions, and resource cleanup.

These experiments can use ordinary jobs and a small service before adopting a
large custom-resource model. They are proposed learning steps, not approved
implementation milestones.

## Measurements

Compare similar tasks with and without prepared Ayni-backed execution. Record
sample size, agent/runtime, model, repository, and task difficulty rather than
attributing every difference to the platform.

| Measure | What it helps assess |
| --- | --- |
| Time to first useful verification | Preparation and onboarding value |
| Environment/setup failure rate | Reliability of the readiness flow |
| Repair iterations and repeated setup commands | Feedback-loop usefulness |
| Cost per reviewed and accepted change | Compute and model economics |
| Reviewer effort and requested rework | Practical quality and explainability |
| Successful clean reproduction | Evidence and environment portability |
| Idle resources and cleanup delay | Operational overhead |

The MVP selects a ten-task pilot; its measurement definitions and decision rules
are recorded in the [MVP validation plan](mvp-validation.md). Numeric performance
targets for this broader backlog have not been selected. More PRs or faster
agent output alone do not demonstrate better accepted changes.

## Go/no-go questions

- Does Ayni materially improve existing-agent workflows, including Cursor?
- Can a team onboard without maintaining a second bespoke development platform?
- Does agent discovery reduce configuration while preserving required checks?
- Do multi-repo results expose a useful integration failure that per-repo checks miss?
- Can maintainers understand and recover failed builds or tasks without reading agent internals?
- Does self-hosted operational ownership matter enough for teams to adopt this?

## Documentation maintenance and publication

Update the decision status as evidence arrives. Promote a proposal into an
accepted decision with rationale and validation results; do not silently rewrite
an experiment as a shipped capability. Recheck external product/library claims
before acting on them.

Before any public launch, make an explicit decision on repository visibility,
licensing, maturity claims, and supported workflows. No public workflows,
announcements, or references from Ayni are part of this documentation milestone.
