<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/site/public/ayni-orchestrator-dark.svg">
  <img src="docs/site/public/ayni-orchestrator.svg" alt="Ayni Orchestrator logo" width="64" height="64">
</picture>

# Ayni Orchestrator

Ayni Orchestrator is rebuilding its project-conversation workflow on a durable PostgreSQL foundation.

The current runnable milestone is **M1 runtime Slice 1**. It provides:

- PostgreSQL 17.6 as the sole workflow authority;
- repository-owned forward migrations under an advisory lock;
- minimal workspace, project, and conversation creation;
- durable, scoped-idempotent message admission with FIFO turn ordering;
- conversation snapshots and stable bounded history;
- a canonical private-v1 gRPC API on a trusted pod-loopback listener;
- a Docker Desktop Helm stack with distinct migration/runtime roles and retained storage.

This is a clean replacement, not a compatibility migration. The prior SQLite workflow, browser/BFF, runtime workers, legacy development fixtures, and custom-client workflow path are not part of the current runtime.

JetStream delivery, executor ownership and results, resumable watches, providers, browser UX, and shared-environment release enter later slices.

## Start locally

```sh
dev/ctl deploy
dev/ctl check --expect-empty
```

See [local development](docs/technical/operations/local-development.md) for prerequisites, trusted diagnostics, validation, restart checks, and explicit cleanup.

## Documentation

- [Architecture](docs/technical/architecture/architecture.md)
- [Operations](docs/technical/operations/operations.md)
- [M1 durable foundation contract](docs/technical/research/project-workflow-m1-durable-foundation.md)
- [Technical documentation index](docs/technical/README.md)
- [Contributor preview](docs/site/index.md)

See [CONTRIBUTING.md](CONTRIBUTING.md) for repository checks, [AGENTS.md](AGENTS.md) for repository rules, and [ROADMAP.md](ROADMAP.md) for later slices.
