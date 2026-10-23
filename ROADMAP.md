# Roadmap

This roadmap records agreed product direction. Requirements, acceptance criteria, evidence, and delivery status belong in the pull request for each bounded slice. Development branches start from current `origin/alpha` and target `alpha`.

## Project workflow replacement

The replacement is clean-slate. PostgreSQL is authoritative transactional state, JetStream will carry durable delivery, and gRPC provides command and observation contracts. Prior SQLite workflow schemas, retained data, APIs, tests, browser paths, and broker topology are research input only: do not migrate, dual-write, preserve, or adapt them.

The [M1 durable foundation contract](docs/technical/research/project-workflow-m1-durable-foundation.md) is binding for M1. The [refactor runbook](docs/technical/research/project-workflow-refactor-runbook.md), [worksheets](docs/technical/research/project-workflow-design-worksheets.md), and [feasibility evidence](docs/technical/research/project-workflow-m1-feasibility-evidence.md) preserve the investigation and approved boundaries.

### Alpha contract policy

Every Orchestrator-owned interface remains v1 until alpha merges. Consolidate
research into the canonical implementation; do not create parallel interface
versions for delivery milestones. Immutable configuration revision IDs, session
generations, ordering revisions and SQL migration ordinals retain their distinct
meaning. The [conversation execution contract](docs/technical/architecture/conversation-execution.md)
owns the consolidated M2 decisions, including provider uncertainty, cancellation
and initial tool authority.

### Product constraints

- Conversations share project configuration but retain isolated working context.
- Plans originate in conversations and require explicit approval of an immutable version.
- Concurrent tasks use isolated worktrees and freeze selected source commits.
- Questions and failures return to their originating conversation.
- Task delivery ends at a reviewable draft; users retain merge authority.
- Workspace instructions and MCP integrations precede multiple providers.
- Tenancy expansion and user management remain outside this replacement.

### Delivery sequence

| Milestone | Observable outcome |
| --- | --- |
| M1: Durable foundation | PostgreSQL persistence, durable delivery, one scripted executor, canonical private-v1 commands, snapshots, history, and resumable observation survive failures. |
| M2: Conversation execution | Shared agent configuration, isolated sessions, workspace instructions, one provider, and structured activity. |
| M3: Plans and approval | Conversation-owned immutable plan versions and exactly-once approval receipts admit the reviewed task set. |
| M4: Isolated task execution | Ayni runtimes, worker supervision, retained evidence, and draft publication expose separate execution, quality, and publication outcomes. |
| M5: Dependencies and concurrency | Concurrent worktrees and explicit upstream commit selection keep dependent inputs frozen. |
| M6: Feedback and recovery | Attention items, retained work, recovery proposals, and confirmation survive pod loss. |
| M7: Project continuity and integrations | Factual summaries, outcome assessment, proactive proposals, MCP integrations, and additional providers respect configured scope. |
| M8: Operational acceptance | Retention, diagnostics, capacity, self-development, cutover, and obsolete-path removal pass cumulative failure tests. |

### M1 runtime slices

M1 is delivered as cumulative, executable slices rather than one unsafe cutover.

1. **PostgreSQL authority and canonical private-v1.** Minimal hierarchy, scoped idempotency, FIFO message admission, exact outbox bytes, snapshots/history, readiness, trusted-loopback APIs, local PostgreSQL, and deletion of superseded callers.
2. **Durable scripted handoff.** Isolated authenticated JetStream streams, a retained scripted executor, signed ownership claims, application report decisions, accepted responses, and FIFO successor promotion survive application, broker, and executor restarts without treating broker storage as ownership.
3. **Resumable observation and cumulative acceptance.** Notification-backed watch replay, quarantine, retention cleanup, metrics, version-skew/capacity evidence, and interruption at every commit/publication/claim/report/decision/ack boundary prove no accepted work or retry bytes are lost.

Slice 3 implements `WatchConversation` using an atomic PostgreSQL snapshot boundary and retained update replay. The cumulative acceptance harness exercises admission, broker delivery, ownership, reports, decisions, replay, capacity, and retained-state recovery. M1 does not deploy providers, browser UX, or custom-client workflow access. Later milestones build on the PostgreSQL and retained handoff authorities.

### M1–M2 delivery scopes

M1's three runtime slices are complete on `alpha`. Four staged PRs bridge local
browser access and deliver M2. Each PR is its own work item: its description owns
requirements, acceptance criteria, failure/recovery cases, and validation evidence.
Do not create a separate issue for these scopes. Each subsequent PR starts from
current `origin/alpha` after its prerequisite merges.

| PR | Scope | Observable outcome and acceptance |
| --- | --- | --- |
| 1: Contributor conversation console | Local browser gateway, canonical bounded hierarchy discovery, creation, message admission, history, activity, and resumable observation. | Exercise the scripted stack from the browser. Refresh, two tabs, disconnect/reconnect, slow readers, cursor expiry, and ambiguous command retries preserve scope and accepted work. |
| 2: Configuration and isolated sessions | Immutable shared agent configuration and workspace instruction revisions within v1; admission-time revisions, model/provider binding and budget; isolated session provisioning, generations, fencing, wake-up, idle expiry, adoption and replacement, initially with scripted execution. Define the complete provider-call/cancellation contract, select one provider from evidence, and agree the initial bounded tool set. | Two conversations share configuration while keeping context, session storage and activity isolated. Later configuration edits do not rewrite admitted turns; replaced executors cannot advance state. |
| 3: Real provider conversations | One provider in isolated sessions, bounded context, retained request intent/identity and result/usage, budget reservation, cancellation, explicit uncertain outcomes, direct scoped credential injection, and console provider/usage views. | Converse with a real provider through the console. An ambiguous paid call is never blindly retried; accepted results survive restarts and appear once. Provider safety ships with the first real calls. |
| 4: Tools and complete M2 acceptance | Agreed bounded tools and authority limits, safe preparation/model/tool activity, remaining M2 recovery controls, and the cumulative M2 acceptance matrix. | Observe permitted tools and actionable failure states. Isolation and interruption tests span preparation, request uncertainty, accepted results, reconnect, session overlap and ledger cleanup. |

The console remains a maintained local contributor surface through M2. It does not
introduce shared-environment access or user management. M3/M4 retain plan approval,
coding worktrees, Ayni task execution, quality evidence, and draft publication; MCP
integrations remain M7. The [M2 handoff contract](docs/technical/research/project-workflow-m1-durable-foundation.md#m2-handoff)
owns the inherited invariants. The [conversation execution contract](docs/technical/architecture/conversation-execution.md) consolidates configuration, session, provider/cancellation and bounded-tool decisions within v1.

M2 conversation execution is implemented: the contributor console, shared frozen
configuration, isolated sessions, one real provider, bounded read tools and explicit
uncertainty recovery share the signed acceptance boundary. Cumulative acceptance
covers deterministic failure fixtures, deployed session interruption and a bounded
live-provider smoke. The delivery PR owns the execution evidence. M3 plans and
approval are implemented through the four delivery slices below; M4 task delivery
builds on that admitted-work boundary.

Every runtime PR extends the cumulative harness and exercises affected behavior on
Docker Desktop Kubernetes. PR 4 includes repeatable provider fixtures plus a bounded
live-provider smoke test. Scripted and fixture timings do not predict real model
latency. PR descriptions record any failed or blocked checks.

### M3 delivery scopes

The [plans and approval contract](docs/technical/architecture/plans-and-approval.md)
defines M3 ownership, immutable review, explicit authorization and atomic task
admission. The [delivery plan](docs/technical/research/project-workflow-m3-delivery-plan.md)
specifies four sequential PRs and their failure/recovery acceptance gates:

| PR | Observable outcome |
| --- | --- |
| 1: Durable plan review | Conversation-owned immutable versions, bounded reads and console review survive reload and concurrent edits. |
| 2: Agent plan proposals | Scope/constraint discussion produces validated, retained structured proposals without granting approval authority. |
| 3: Approval and task admission | Explicit approval of one exact version atomically retains one receipt and the complete reviewed task set. |
| 4: Cumulative M3 acceptance | Deployed interruption, retry, race, isolation and observation evidence closes the milestone. |

Every admitted task retains workspace, project, originating conversation, plan
version and approval receipt as immutable, constrained references. M4 carries
that origin into attempts and reports. M6 routes questions/failures back to the
originating conversation; M7 uses accepted outcomes for factual project continuity
and bounded assessment without sharing private working context.

M3 admits tasks in an awaiting-execution state; it does not dispatch coding work.
One plan admits one approved task set. Follow-up work requires a new linked plan
until later confirmed recovery defines replacement of admitted work. PR #238 combines
the contract with the first operator-authored durable-review slice. The second
slice adds [structured agent proposals](docs/technical/architecture/agent-proposals.md)
and retained provenance. The third slice adds [approval and task admission](docs/technical/architecture/task-admission.md): exact reviewed-version authorization, one retained receipt and the complete task set, with immutable origin and visible execution blockers. The fourth slice adds the [cumulative M3 acceptance gates](docs/technical/operations/plan-acceptance.md), including broker-outage approval and exact retained-state recovery through workload restarts. Milestone completion requires passing deployed evidence in the final delivery PR; each delivery PR owns its acceptance evidence and status changes.

### M4 delivery scopes

M3 delivery slices are implemented through PR #241. The
[M4 delivery plan](docs/technical/research/project-workflow-m4-delivery-plan.md)
defines six sequential PRs:

| PR | Observable outcome |
| --- | --- |
| 1: Execution readiness and attempts | Trusted local source verification, frozen commit/lock/configuration, selected runtime and authorized budgets produce one prepared attempt or retained blockers, without dispatch. |
| 2: Isolated coding execution | One task executes in isolation with durable ownership, fencing, cancellation and retained changes. |
| 3: Ayni quality and retained evidence | Actual worker runtime verification and frozen validation produce separate quality outcomes and trustworthy artifacts. |
| 4: Draft publication | Accepted changes produce one recoverable reviewable draft under separate publication authority. |
| 5: Operator visibility and controls | Complete attempt, progress, evidence and execution/quality/publication views and bounded controls. |
| 6: Cumulative M4 acceptance | Deployed interruption, restart, fencing, cancellation and cleanup close the milestone. |

The [execution preparation contract](docs/technical/architecture/task-execution.md)
owns the first-slice authority and frozen-input boundary. PR #242 delivers that
preparation boundary. The [isolated execution contract](docs/technical/architecture/isolated-task-execution.md)
defines the second slice: a durable serial task queue, isolated retained checkouts,
bounded coding, cancellation fencing and exact result acceptance, delivered by PR #243.
The [task quality contract](docs/technical/architecture/task-quality.md) owns the
third slice: credential-free managed validation, frozen inputs and retained
evidence separate from coding completion, delivered by PR #244.
The [draft publication contract](docs/technical/architecture/task-publication.md)
defines the fourth slice: explicit publication authority, deterministic Git
identity and retained draft recovery, delivered by PR #245.
The [console task contract](docs/technical/architecture/contributor-console.md#task-observation-and-publication-controls)
defines the fifth slice: bounded task inspection, separate outcome/evidence views,
cancellation and explicit reviewed draft publication, delivered by PR #246.
The [task acceptance matrix](docs/technical/operations/task-acceptance.md) owns
the sixth slice: deployed late-worker fencing, publisher loss around remote
creation, retained uncertainty and cleanup/redeployment with exact evidence replay.
Each runtime PR extends the cumulative acceptance harness immediately. M4 begins with serial execution
and isolated attempts; dependency scheduling and concurrent tasks remain M5,
conversational recovery remains M6. Automatic quality-repair loops require explicit
bounded authorization before implementation. M4's six delivery slices are implemented. The final delivery PR records
executed cumulative closure evidence; the local profile uses scripted coding, real managed
quality and Git transport, and an explicit GitHub protocol fixture.

### Validation at every milestone

| Level | Evidence |
| --- | --- |
| Contract | Valid transitions, exact API surface, scoped identities, and frozen inputs. |
| Integration | Real storage/transport, migrations, retries, and projections. |
| Failure | Controlled interruption at every durability boundary. |
| Operator scenario | Observable command, progress, reconnect, recovery, and cleanup behavior. |

Keep one cumulative acceptance harness. A passing feasibility experiment does not make its mechanism production runtime behavior. Each delivery PR names prerequisites, observable outcomes, failure cases, validation commands, and obsolete paths to delete.

Ayni's independent CLI roadmap belongs in its own repository.
