# Technical documentation

These documents support contributors, deployment maintainers and coding agents.
They are repository sources, excluded from the human-facing documentation site.
For product usage, start with the [user guide](../site/index.md).

## Understand the implementation

- [Architecture](architecture/architecture.md): control plane, worker and persistence.
- [Code ownership](architecture/code-organization.md): shared libraries and dependency direction.
- [Event platform](architecture/event-platform.md): reporting, acceptance, delivery and cleanup.
- [Worker](architecture/runtime-runner.md): budgets, sandbox, checks and publication.
- [Assurance](architecture/assurance.md): current trust boundaries and research limits.
- [UI design system](architecture/design-system.md): reusable components and presentation rules.

## Maintain a deployment

- [Local development](operations/local-development.md): Helm, Tilt and Docker Desktop validation.
- [Operations](operations/operations.md): schema upgrades, credentials and recovery.
- [Event compatibility](operations/event-compatibility.md): reader drain and rollback gates.
- [GitHub security](operations/github-security.md): provider and repository permissions.
- [Managed execution](guides/managed-runtime-execution.md): build and activation internals.

## Integrate with contracts

- [Ayni](reference/ayni-integration.md) and [official builders](reference/repository-builders.md).
- [Custom clients](reference/custom-client-access.md) and [protocol tooling](../../proto/README.md).
- [Verification](reference/verification-events.md), [runtime operations](reference/runtime-operation-events.md), and [coding](reference/coding-events.md).
- [Workflow traces and evaluations](reference/workflow-evaluations.md): repeatable scenarios and retained comparisons.
- [Operator projections](reference/operator-projections.md), [CloudEvents](reference/cloudevents-contract.md), and [observability](reference/observability-and-events.md).

[Documentation authoring](contributing/documentation.md) defines the audience boundary.
[Decisions](decisions/README.md) and [dated research](history/event-platform/index.md)
preserve provenance. The [roadmap](../../ROADMAP.md) links remaining work.
