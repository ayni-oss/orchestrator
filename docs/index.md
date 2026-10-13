# Ayni Orchestrator

A control plane for development tasks that use verified Ayni runtimes.
One worker owns coding, quality checks and draft publication; the control plane
admits work, accepts evidence and coordinates recovery.

## Start here

- [Local development](operations/local-development.md): bring up the Docker Desktop Kubernetes environment.
- [Operator onboarding](guides/operator-onboarding.md): connect GitHub, GHCR and signing certificates.
- [Product flow](guides/ayni-product-flow.md): understand repositories, runtimes, projects and tasks.
- [Managed execution](guides/managed-runtime-execution.md): build, verify and execute a repository runtime.

## Understand and operate

- [Architecture](architecture/architecture.md): execution and library boundaries.
- [Assurance](architecture/assurance.md): what certification and quality evidence establish.
- [Operations](operations/operations.md): upgrades, retries, retained data and cleanup.
- [Event platform](architecture/event-platform.md): reporting, acceptance, delivery and projections.
- [Observability](reference/observability-and-events.md): logs, metrics and tracing.
- [Ayni integration](reference/ayni-integration.md): supported CLI and artifact contracts.
- [Custom clients](reference/custom-client-access.md): access and transport boundaries.

## Contribute

[Contributing](../CONTRIBUTING.md) describes validation and the PR workflow.
[Code organization](architecture/code-organization.md) identifies the owner of each layer.
[Documentation authoring](contributing/documentation.md) explains the site and source layout.
The [roadmap](../ROADMAP.md) links remaining work; [decisions](decisions/README.md)
and [dated research](history/event-platform/index.md) retain historical context.

The CLI has its own [documentation](https://cli.ayni.build).
