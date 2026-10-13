<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/public/ayni-logo-dark.svg">
  <img src="docs/public/ayni-logo.svg" alt="Ayni icon" width="64" height="64">
</picture>

# Ayni Orchestrator

Ayni Orchestrator is a control plane for worker-owned development tasks.
Integrations manage provider access, repositories own shared runtimes, and projects
organize work. A task freezes its source, runtime digest, model, constraints,
coding budget and deadline before one worker executes it.

The worker attests its runtime, checks out source, codes, commits, runs full Ayni
quality checks with bounded repairs, retains evidence and publishes a draft PR.
Execution, quality and publication have independent outcomes. Publication retry
uses retained final work without starting another coding session.

The control plane admits and schedules work, supplies scoped credentials, accepts
validated results and handles cleanup. Ayni remains independently useful as a
local CLI. Ayni Managed is supported; Custom Runtime and coordinated
multi-repository work remain future work.

## Documentation

Start with the [documentation index](docs/index.md) or the site at
[orchestrator.ayni.build](https://orchestrator.ayni.build).

- [Local development](docs/operations/local-development.md): run the local Helm/Tilt topology.
- [Operator onboarding](docs/guides/operator-onboarding.md): configure integrations, certificates and runtimes.
- [Product flow](docs/guides/ayni-product-flow.md): repository, project and task ownership.
- [Architecture](docs/architecture/architecture.md): code boundaries and implemented execution.
- [Operations](docs/operations/operations.md): upgrades, recovery and cleanup.
- [Protocol](proto/README.md): source schemas and generated clients.

## Contributing and delivery

See [CONTRIBUTING.md](CONTRIBUTING.md) for checks and documentation authoring,
[AGENTS.md](AGENTS.md) for repository rules, and [ROADMAP.md](ROADMAP.md) for the
project and remaining work. Completed delivery lives in Git history, merged PRs
and closed issues. [Historical decisions](docs/decisions/README.md) preserve
provenance; they do not override current guides.
