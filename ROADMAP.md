# Roadmap

Deliver an Ayni-first software factory through project chat, reviewed plans and
autonomous task execution. Alpha finishes at reviewable pull requests with passing
Ayni quality and evidence for the requested behavior. It does not merge or deploy.

The [alpha product contract](docs/technical/architecture/alpha-product.md) owns the
agreed scope, authority and completion rules. This roadmap owns delivery order and
milestone acceptance; each PR owns its implementation status and execution evidence.
Start fresh branches from current `origin/alpha` and target `alpha`.

## Alpha scope

- Connect Ayni-configured repositories and reuse or build, certify, publish and verify their locked runtime images.
- Configure a workspace's available repositories, instructions, contextual resources, skills and MCP capabilities. Projects own goals and model settings; conversations select task repository scope from workspace availability.
- Use project chat to research source, develop and validate plans, follow progress, answer questions and review results.
- Review modular tasks that run independently, sequentially or as a dependency DAG. The plan chooses how tasks map to pull requests within each repository.
- Authorize agents to inspect, edit, add and delete files, update dependencies and locks, write tests, build, run checks and repair failures within scope and allowance.
- Coordinate durable work through events while preserving PostgreSQL authority, frozen inputs, explicit permissions and recoverable external effects.
- Return linked tasks, commits, quality evidence and reviewable PRs to the project. Humans retain merge authority.

Ayni owns tool integrations, environment/lock semantics and quality evaluation.
Orchestrator consumes those public contracts and owns agent workflow, runtime
publication, isolation and delivery evidence. Ayni is the first supported runtime
path; custom containers are future scope, not an alpha support claim.

## Starting point and gaps

The backend contains durable conversations, exact plan approval, isolated attempts,
managed quality, recovery, dependency selections, publication and execution history.
The current UI covers configuration, hierarchy, repository catalog/association,
static Ayni inspection and qualified-profile-gated runtime preparation.

The complete alpha journey is not delivered. Projects inherit workspace repository membership and have independent model configuration. Planning lacks built-in
repository research; coding is one bounded file-proposal pass; task preparation
uses a trusted local helper. Chat/task browser journeys and their cumulative
acceptance need replacement coverage.

Runtime qualification is deployment-specific. The default Docker Desktop profile
remains disabled after failed network-policy enforcement. The separate native
arm64 Cilium profile has Node component and supervisor lifecycle evidence; live
GitHub acquisition, private GHCR happy-path publication and the broader Rust
workload envelope still need qualification. See
[runtime packaging](deploy/images/runtime-preparation/README.md) for measured limits
and exact artifact prerequisites, including the admitted Ayni 0.17.1 release with the Podman identity fix.

The [M1–M8 foundation roadmap](docs/technical/history/backend-foundation-roadmap.md)
is historical provenance. Its archival closes a planning phase, not product
acceptance. Preserve useful mechanisms and evidence; qualify changed source and
restore missing product capabilities rather than repeating a foundation programme.

## Delivery sequence

Each milestone is an observable product increment, delivered through small PRs
with UI, canonical backend operations, safe failures and focused local evidence.
A single-task checkpoint does not reduce alpha's required DAG scope. Event,
context, telemetry and recovery work ship inside these increments.

| Milestone | Dependencies | Maintainer can do | Completion evidence |
| --- | --- | --- | --- |
| A: Workspace scope and execution-ready repositories | Existing catalog, connections and runtime stages | Make catalog repositories available to a workspace; make every workspace repository available to project research; prepare source and a verified runtime through the product. | Workspace membership and task subset checks, shared catalog reuse, exact source/lock/image evidence, real registry readback and server-side source preparation without per-task terminal commands. |
| B: Grounded project chat and plans | A for repository scope; research can proceed while runtime preparation is blocked | Research selected repositories, retrieve project context, apply configured skills/MCP tools and review a versioned plan with a task graph and PR layout. | Real repository reads/search, retained context provenance, isolated project scope, graph validation, exact approval/retry and visible preparation blockers. |
| C: Autonomous task to reviewable PR | A and B | Approve bounded execution and publication, then follow an agent implementing, writing tests and repairing one real task. | Tool-driven multi-round work, dependency/lock changes where required, managed Ayni pass, acceptance evidence and one recoverable real PR; event-backed dispatch/report/cancellation and browser recovery. |
| D: Complete project graphs and delivery | C; graph design starts in B | Run standalone, sequential and branching tasks, parallelize ready work, inspect blockers and deliver the plan's chosen PR structure. | Dependency and integration evidence, explicit upstream commits, conflicting changes, failed-upstream blocking, independent-task progress, shared/separate PR linkage and confirmed recovery without duplicate execution/publication. |
| E: Alpha qualification | A–D | Complete a realistic multi-task project from repository setup and chat to reviewable PRs, including a real change to Orchestrator. | Actual provider, source/registry/forge operations, managed Ayni quality, requested-behavior evidence, current browser coverage and replay across owner restarts/cancellation. Record interventions, resource envelope and all unmet gates. |

### A: Workspace scope and execution-ready repositories

Keep the catalog shared and repository identities independent of workspaces.
Workspace settings manages catalog availability, and project attachment enforces the workspace subset.
Reuse the same repository and verified artifact across workspaces only after each
workspace makes it available; never share project-private context or authority.

Complete runtime acquisition, registry publication and verification on the admitted
profile. Connect those artifacts to trusted server-side Git/source preparation,
including dependency ancestry, then freeze each admitted attempt. A configuration
edit or removal blocks new use as appropriate but cannot rewrite existing attempts.

Repository onboarding should retain reviewed settings and carry one bounded run from the latest default branch through inspection, build, certification, publication and verification. Keep current progress, deployment blockers and the confirmed registry image visible; later runs may reuse reviewed settings without granting recurring builds.

Show runtime progress as five onboarding steps, with technical evidence under
details. The first UI increment adds this progress view and a browser-generated
signing key backup. The branch check can bind a run to the exact reviewed commit and integration
revisions. Repository onboarding reuses the shared GitHub App, registry and
signer setup forms. Continue with server-retained progress before review, then
add reviewed trust PRs and merge reconciliation.

The [repository delivery plan](docs/technical/research/repository-runtime-workspace-delivery.md)
details setup and handoff scenarios. Missing support, denied access, invalid trust,
outages and cleanup must remain actionable states rather than fallback execution.

### B: Grounded project chat and plans

Project chat supports continuing project progress, not just a single prompt/response.
Research tools read and search the selected repositories and retrieve earlier
project decisions and evidence with provenance. Define bounded context selection,
retrieval and compaction without silently losing original records.

Workspace instructions, context, versioned skills and scoped MCP grants are real
execution inputs. Projects select/refine them within workspace authority. Freeze
the revisions used for planning and execution; importing a skill grants no tool
permission. Qualify one actual MCP integration and skill-driven scenario.

Plans contain objectives, acceptance criteria, modular task specifications,
dependency edges, repository scope, environment impact, execution/publication
allowances and the chosen PR grouping/base relationships. Approval binds an exact
version. An approved plan can authorize subsequent ready tasks within those grants;
users need not approve each ordinary tool call or repair round.

### C: Autonomous task to reviewable PR

Replace the one-shot file-proposal limitation with a bounded engineering loop.
Give agents source research, file operations, command execution, test authoring,
build/test results and iterative repair. Apply reviewed scope and resource limits
without making a fixed file count the product's engineering capability boundary.

Handle dependency and Ayni lock changes through trusted validation and a matching
environment preparation path. Keep previous snapshots/evidence immutable and
retain candidate revisions before external effects. Workers cannot loosen policy
to silence findings or certify their own runtime/evidence.

Extend existing durable delivery for task commands, reports, authorization,
cancellation and decisions; replace the corresponding polling/HTTP ownership path.
Keep broker credentials outside the coding process. See the
[task event direction](docs/technical/architecture/task-events.md).

Ordinary implementation and repair consume the approved allowance without another
confirmation. Questions, scope changes, exhausted allowance and uncertain external
effects return to project chat with retained work and an explicit next action.
Publication may consume a grant reviewed with the plan, but remains a distinct
accepted operation and outcome. Finish at a reviewable PR; never merge or deploy.

### D: Complete project graphs and delivery

Preserve modular tasks and distinguish logical dependencies from PR layout.
A task can run alone, be sequenced or have multiple upstream/downstream edges.
The plan can group tasks into one repository PR or use separate/stacked PRs; tasks,
attempts, exact commits and PR relationships remain navigable in either case.

Prepare downstream source from explicitly selected accepted upstream outcomes.
Moving branches, upstream retries or a new plan cannot silently rebase an active
attempt. Integrate same-repository changes in isolation, resolve conflicts within
authorized work, and require amended review when the intended scope changes.

Cross-repository edges retain exact source/artifact or contract references and the
checks that establish compatibility. A Git ancestry check alone proves neither
cross-repository compatibility nor an integrated product result. Each PR belongs
to one repository; a project may have several linked PRs.

Validate the exact assembled source of each PR group with managed Ayni quality
and its acceptance checks. Passing task checks do not establish integration success.

Show graph readiness, running tasks, blocked edges, questions and delivery status
alongside chat. Accepted task outcomes update factual project progress; an agent
summary never substitutes for quality, acceptance evidence or human merge review.

### E: Alpha qualification

Use the current sole web application, canonical APIs and actual admitted workload
profile. Cover setup, planning, approval, single/sequential/diamond graphs,
independent concurrency, environment-changing work, tests, repair, PR layout and
project feedback. Exercise at least one real Orchestrator change and a project
that selects multiple repositories from a workspace.

Interrupt browser connections and execution owners at meaningful boundaries.
Prove scope isolation, exact command recovery, stale-generation rejection,
provider uncertainty, failed upstreams, cancellation, cleanup and publication
readback. A broker ACK, fixture patch or passing host check cannot close a product gate.

Publish the admitted Ayni integration support matrix with exact project shapes,
platform/tool combinations, execution evidence and blocked or unsupported cases.

Record three separate facts for each capability: backend implemented, usable in
the product, and qualified on the declared deployment. Alpha requires all three
for every in-scope journey. Existing retired browser profiles must be replaced or
explicitly superseded by runnable coverage, not bypassed to report success.

## Acceptance and delivery discipline

- A task needs passing managed Ayni quality and evidence for its requested behavior. Missing/incomplete required evidence is a blocker; no policy weakening to achieve a green result.
- Keep execution, quality, acceptance, cleanup and publication outcomes separate. Explain stage, waiting reason, allowance and permitted next action through the UI.
- Preserve exact approval, source/runtime/context revisions, retained receipts, ownership fencing and recoverable external effects. PostgreSQL remains authoritative.
- Keep interfaces canonical v1 during alpha. Update source contracts, generated consumers and affected docs together when implementing a contract change.
- Use bounded event delivery with backpressure and periodic reconciliation. Large sources/evidence use immutable references; provisional text is not accepted outcome evidence.
- Include causal history, safe traces and resource/usage observations in each slice. Never export credentials or use sampled telemetry as project accounting.
- Run focused local browser, PostgreSQL, JetStream, loopback and deployed acceptance for affected behavior. PR CI stays lean; reintroducing end-to-end CI requires a separate scope/runtime decision.
- Each PR links a milestone, states what users can do, records passed/failed/blocked checks and removes superseded active paths only with replacement evidence. Do not create duplicate issue trackers or merge automatically.

## After alpha

- Repository events, including GitHub issues, can initiate a project prompt under explicit trigger policy. Milestone links, schedules and other inputs build on the same origin, deduplication and authority contracts.
- Custom containers require their own execution, verification and support contract.
- Rich project/workspace analytics, broader provider administration, remote tenancy and multi-user access follow defined product journeys.
- Design queue, workspace/provider budgets and dispatch rates for growth; thousands of agents and sustained production capacity require measured load qualification, not a transport claim.
- Automatic merge, deployment and broad business-outcome measurement remain outside alpha.

Ayni's [independent roadmap](https://github.com/ayni-oss/ayni/blob/main/ROADMAP.md)
owns its tool, environment and compatibility evolution. Orchestrator-specific
workflow requirements belong here.

## Execution capture and debug deployments

Deliver shared process supervision and acknowledged worker telemetry, then collect
all managed container stdout/stderr through the Kubernetes API, expose retained
history in the local debug console, and qualify the complete deployment. Raw logs
are deployment-gated and separate from safe diagnostics. Preserve existing
workflow authority and isolated verifier boundaries; report API rotation/deletion
uncertainty and storage loss explicitly. See the [capture contract](docs/technical/architecture/execution-capture.md).

The first slice adds the shared runner, canonical telemetry contract, durable
journals, PostgreSQL replay and runtime-build integration. Collection, the local
frontend and complete deployed qualification remain subsequent review slices.

The second slice integrates task/publication command capture, namespace-scoped
Kubernetes API collection and abandoned-spool replay. Deployment activation and
frontend availability follow in slice 3; failure qualification remains a gate.
