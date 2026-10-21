# Technical documentation

The current implementation is M1 runtime Slice 3: PostgreSQL-authoritative workflow state, authenticated durable delivery, one retained scripted executor, and a trusted-loopback private-v1 API.

## Current implementation

- [Architecture](architecture/architecture.md): authority, transactions, API surface, projections, and failure boundaries.
- [Code organization](architecture/code-organization.md): production ownership and dependency direction.
- [Local development](operations/local-development.md): Docker Desktop deployment, diagnostics, validation, restart, and cleanup.
- [Operations](operations/operations.md): startup, probes, schema changes, credentials, backup, and failure handling.
- [Protocol tooling](../../proto/README.md): canonical protobuf generation and compatibility policy.

## Binding research

- [M1 durable foundation](research/project-workflow-m1-durable-foundation.md): full milestone contract and later-slice handoffs.
- [M1 acceptance evidence](research/project-workflow-m1-acceptance-evidence.md): executable gates, reproduction, and coverage limits.
- [Feasibility evidence](research/project-workflow-m1-feasibility-evidence.md): closed pre-implementation evidence.
- [Refactor runbook](research/project-workflow-refactor-runbook.md) and [design worksheets](research/project-workflow-design-worksheets.md): investigation provenance.

The [event-platform history](history/event-platform/index.md) is retained as dated evidence, not as current runtime documentation. Superseded browser, SQLite workflow, runtime-worker, provider, compatibility-drain, and custom-client instructions were removed during the clean cutover.

[Documentation authoring](contributing/documentation.md) defines the audience boundary. The [roadmap](../../ROADMAP.md) tracks later slices.

- [Contributor console](architecture/contributor-console.md): local browser transport, discovery, and retry boundaries.
