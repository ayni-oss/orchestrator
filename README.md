<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/site/public/ayni-orchestrator-dark.svg">
  <img src="docs/site/public/ayni-orchestrator.svg" alt="Ayni Orchestrator logo" width="64" height="64">
</picture>

# Ayni Orchestrator

Ayni Orchestrator is rebuilding its project-conversation workflow on a durable PostgreSQL foundation.

The runtime provides **conversation execution, plan approval and isolated task delivery through managed quality and draft publication** on the M1 durable foundation, with a local contributor console, shared agent configuration and isolated sessions supporting scripted or explicitly enabled OpenAI execution. It provides:

- PostgreSQL 17.6 as the sole workflow authority;
- conversation-owned operator and agent plans with immutable versions, scoped history, retained provenance, exact write receipts and console review;
- explicit approval of an exact plan version/digest with atomic, once-only admission and immutable task origin;
- repository-owned forward migrations under an advisory lock;
- minimal workspace, project, and conversation creation;
- durable, scoped-idempotent message admission with FIFO turn ordering;
- an isolated authenticated three-stream JetStream profile;
- exact-byte command and decision publication with broker storage kept separate from ownership;
- a retained scripted executor with signed claim, acceptance, and terminal-report handoffs;
- accepted scripted responses, FIFO successor promotion, snapshots, and stable bounded history;
- resumable notification-backed watches with durable replay and typed cursor expiry;
- bounded consumer quarantine, replay retention cleanup, and operational metrics;
- immutable workspace instructions, provider/model settings and token budgets frozen at message admission;
- conversation-scoped session generations, retained result ledgers, idle expiry and replacement;
- bounded foreground OpenAI Responses requests with retained identity, reservation, result and nullable usage;
- durable turn cancellation and explicit uncertain outcomes that never automatically retry paid calls;
- three scoped read tools with frozen inputs, retained invocation results and turn-wide budgets;
- explicit possible-charge acknowledgement to retire a session held by an uncertain call;
- a canonical private-v1 gRPC API on a trusted pod-loopback listener; and
- a Docker Desktop Helm stack with separated database/broker identities and retained storage.

This is a clean replacement, not a compatibility migration. The prior SQLite workflow, browser/BFF, runtime workers, legacy development fixtures, and custom-client workflow path are not part of the current runtime.

The cumulative M1 acceptance evidence is recorded in the delivery PR. The local contributor console exposes shared configuration, isolated sessions, provider usage and cancellation. OpenAI execution requires an explicit model, deployment enablement and a workspace-scoped credential mounted directly into its session. Scripted execution remains the default. OpenAI conversations can read frozen workspace instructions, project status and their own bounded history. The final M2 acceptance scope includes deterministic provider/tool recovery fixtures and a separately reported live-provider smoke test. All Orchestrator interfaces stay v1 until alpha merges.

Plan review retains the project and originating conversation for later task feedback. Structured agent proposals retain provider and conversation provenance; explicit approval atomically admits the reviewed task set in an awaiting-execution state. Plan creation and approval dispatch no coding work. Trusted operators can prepare a task with frozen local Git evidence, runtime selection and budgets; preparation retains blockers or one attempt and also dispatches no coding work. Prepared tasks can be explicitly started as isolated, serial coding attempts with retained changes and cancellation fencing. A credential-free quality stage validates a frozen reconstruction with managed Ayni evidence and exposes quality separately from coding completion. See [task quality](docs/technical/architecture/task-quality.md). The console Tasks view shows progress, blockers, accepted patches and quality artifacts separately, with bounded cancellation and reviewed publication controls. Source verification and upload use the trusted local helper. Operators can separately authorize accepted changes for a recoverable GitHub draft under scoped publication authority. See [draft publication](docs/technical/architecture/task-publication.md). See [isolated execution](docs/technical/architecture/isolated-task-execution.md). See [execution preparation](docs/technical/architecture/task-execution.md). See [durable plan review](docs/technical/architecture/plan-review.md).

Task execution and quality problems also produce retained feedback in their
originating conversation and project attention. Notices link to task evidence;
they do not authorize continuation or repair. See [task feedback](docs/technical/architecture/task-feedback.md)
and the [M6 delivery plan](docs/technical/research/project-workflow-m6-delivery-plan.md)
for the boundary between attention and later confirmed recovery.
[Recovery review](docs/technical/architecture/task-recovery.md) lets operators record
immutable proposals with frozen saved-work evidence and additional allowances, plus
task questions and one answer per question. Recording them authorizes no execution.
Exact confirmation can now authorize one bounded linked attempt with preserved saved
changes and parent evidence. Confirmed recovery resolves the parent attention decision while
conversation history links to the continued coding and quality outcomes. Questions retain
one canonical answer across pagination and concurrent tabs. Dependency-aware continuation
still requires M5; structured worker questions remain outside the coding protocol.

Fresh conversations can read bounded task lifecycle facts and accepted outcomes
through frozen project status. Coding, managed quality and publication remain separate facts; the snapshot
contains no other conversation's transcript, plan text or reasoning. Later task
changes do not rewrite an eligible turn's retained inputs. See
[factual project continuity](docs/technical/architecture/project-continuity.md) and
the [M7 delivery plan](docs/technical/research/project-workflow-m7-delivery-plan.md).
Assessment, proactive proposals, MCP integrations and additional providers remain
later delivery slices.

## Start locally

```sh
dev/ctl deploy
dev/ctl check --expect-empty
dev/ctl console
```

Open `http://127.0.0.1:3000` while the console command runs.

See [task delivery acceptance](docs/technical/operations/task-acceptance.md) for the cumulative M4 gates and integration limits. See [local development](docs/technical/operations/local-development.md) for prerequisites, trusted diagnostics, validation, restart checks, and explicit cleanup.

## Documentation

- [Architecture](docs/technical/architecture/architecture.md)
- [Operations](docs/technical/operations/operations.md)
- [M1 durable foundation contract](docs/technical/research/project-workflow-m1-durable-foundation.md)
- [Technical documentation index](docs/technical/README.md)
- [Contributor preview](docs/site/index.md)

See [CONTRIBUTING.md](CONTRIBUTING.md) for repository checks, [AGENTS.md](AGENTS.md) for repository rules, and [ROADMAP.md](ROADMAP.md) for later slices.
