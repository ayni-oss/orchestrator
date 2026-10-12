# Ayni Orchestrator

Ayni Orchestrator is a control plane for worker-owned development tasks.
Integrations manage provider access, repositories own shared runtimes, and projects
organize repository work. Tasks freeze source, runtime digest, model, constraints,
coding turns, quality repair limit and
deadline. Ayni remains independently useful as a local CLI.

The trusted operator UI onboards one GitHub App, multiple GHCR connections, and
global Ed25519 signing certificates. Ayni runtimes select a certificate or follow
the global default; projects inherit their repositories’ runtime selections.
Kubernetes Secrets hold credentials; SQLite schema version 12 stores safe metadata
and references, preserving retained history and migrating active runtime preparation to typed phases.
Administrator-managed connections remain read-only, while
validated UI replacements activate without restarting the control plane. Registry
probe Jobs test access, and each runtime profile selects its image destination.
Repository image preparation automatically builds a missing environment, certifies
and publishes every successful build, then independently verifies its digest.
Existing-image mismatches require explicit Rebuild. No environment workflow
installation or node-local distribution participates in readiness.
See [operator onboarding](docs/operator-onboarding.md) for the supported flow.

One worker attests its runtime, checks out source, codes and makes coherent
commits, runs mandatory full quality checks with bounded repairs, freezes final
evidence and publishes a truthful draft. The control plane admits, schedules,
supplies credentials, observes progress, collects results and cleans up.
Execution, quality and publication are separate outcomes. A draft with failed
quality is completed delivery with failed quality. Publication retry resumes
retained final work without another coding session.

Repository changes reconcile shared runtimes for future work across consuming projects;
admitted tasks retain their frozen identities. Ayni Managed is operational; Custom
Runtime and coordinated multi-repository tasks remain future work.

The [Polish Ayni development flow milestone](https://github.com/ayni-oss/orchestrator/milestone/8)
tracks documentation, isolated-execution research, and reviewed environment changes.
Separate task-stage pods and independent assessment are research, not current execution guarantees.

## Documentation

- [Ayni product flow and object relationships](docs/ayni-product-flow.md)
- [Architecture](docs/architecture.md)
- [Control-plane events, evidence and streaming research](docs/control-plane-events.md)
- [Observability and lifecycle events](docs/observability-and-events.md)
- [Runtime verification event contract](docs/verification-events.md)
- [CloudEvents lifecycle contract](docs/cloudevents-contract.md)
- [Event platform implementation proposal](docs/event-platform-implementation.md)
- [Event platform migration and retirement](docs/event-platform-migration.md)
- [Assurance boundaries](docs/assurance.md)
- [Reviewed environment changes](docs/environment-changes.md)
- [Protocol contract](proto/README.md)
- [Runtime worker](docs/runtime-runner.md)
- [Managed runtime execution](docs/managed-runtime-execution.md)
- [Repository-selected official builders](docs/repository-builders.md)
- [Operator onboarding](docs/operator-onboarding.md)
- [Operations and breaking cutover](docs/operations.md)
- [Local development](docs/local-development.md)
- [Ayni integration contract](docs/ayni-integration.md)
- [Custom-client access](docs/custom-client-access.md)

[Historical decisions](docs/decisions/README.md) remain as provenance. Descriptions of superseded task
execution in those decisions do not define the active implementation.

## Planning and delivery

Requirements and delivery status live in GitHub rather than duplicate planning
documents:

- [Ayni Orchestrator project](https://github.com/orgs/ayni-oss/projects/2)
- [Orchestrator issues](https://github.com/ayni-oss/orchestrator/issues)
- [Completed milestones](https://github.com/ayni-oss/orchestrator/milestones?state=closed)

Repository development rules and the boundary with Ayni are recorded in
[AGENTS.md](AGENTS.md). This repository is public. New releases, packages,
images, sites and announcements require a separate maintainer decision.
