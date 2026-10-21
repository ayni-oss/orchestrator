# Technical documentation

The runtime combines the M1 durable foundation with shared immutable agent configuration and isolated conversation sessions with scripted and opt-in OpenAI execution. PostgreSQL owns workflow and session authority; authenticated durable delivery and the trusted-loopback API remain v1 through alpha. Real provider calls retain request identity, uncertainty and nullable usage; bounded tools and complete M2 acceptance remain the next scope.

## Current implementation

- [Architecture](architecture/architecture.md): authority, transactions, API surface, projections, and failure boundaries.
- [Provider requests](architecture/provider-requests.md): foreground OpenAI request lifecycle, reservation, cancellation and uncertain-outcome recovery.
- [Conversation execution](architecture/conversation-execution.md): consolidated M2 contract, alpha v1 policy, frozen configuration, isolated sessions, provider uncertainty and tool authority.
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
