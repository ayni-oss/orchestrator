<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/site/public/ayni-orchestrator-dark.svg">
  <img src="docs/site/public/ayni-orchestrator.svg" alt="Ayni Orchestrator logo" width="64" height="64">
</picture>

# Ayni Orchestrator

Ayni Orchestrator is rebuilding its project-conversation workflow on a durable PostgreSQL foundation.

The runtime provides **M2 conversation execution and M3 plan review, agent proposals and task admission** on the M1 durable foundation, with a local contributor console, shared agent configuration and isolated sessions supporting scripted or explicitly enabled OpenAI execution. It provides:

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

Plan review retains the project and originating conversation for later task feedback. Structured agent proposals retain provider and conversation provenance; explicit approval atomically admits the reviewed task set in an awaiting-execution state. Plan creation and approval dispatch no coding work. Trusted operators can prepare a task with frozen local Git evidence, runtime selection and budgets; preparation retains blockers or one attempt and also dispatches no coding work. See [execution preparation](docs/technical/architecture/task-execution.md). See [durable plan review](docs/technical/architecture/plan-review.md).

## Start locally

```sh
dev/ctl deploy
dev/ctl check --expect-empty
dev/ctl console
```

Open `http://127.0.0.1:3000` while the console command runs.

See [local development](docs/technical/operations/local-development.md) for prerequisites, trusted diagnostics, validation, restart checks, and explicit cleanup.

## Documentation

- [Architecture](docs/technical/architecture/architecture.md)
- [Operations](docs/technical/operations/operations.md)
- [M1 durable foundation contract](docs/technical/research/project-workflow-m1-durable-foundation.md)
- [Technical documentation index](docs/technical/README.md)
- [Contributor preview](docs/site/index.md)

See [CONTRIBUTING.md](CONTRIBUTING.md) for repository checks, [AGENTS.md](AGENTS.md) for repository rules, and [ROADMAP.md](ROADMAP.md) for later slices.
