# Ayni Orchestrator

Ayni Orchestrator is a private development-workflow control plane that produces
reviewable repository changes on controlled infrastructure. Its pre-public v1
is being replaced by one shared foundation with **Ayni Managed** and **Custom
Runtime** runtime profiles. Ayni Managed is the implemented execution path;
Custom Runtime remains a follow-up. The agreed product journey is
**Connectors → Projects → Tasks**, defined in
[Decision 0009](docs/decisions/0009-connectors-projects-and-internal-assessments.md),
which retains the shared runtime foundations from
[Decision 0007](docs/decisions/0007-clean-v1-runtime-profiles.md).

The implementation still includes repository-first onboarding and Actions
assessment collection. The accepted replacement flows are tracked by
[the product epic](https://github.com/ayni-oss/orchestrator/issues/90); see the
[architecture](docs/architecture.md) for current ownership and remaining gaps.
This repository remains private; Ayni is independently useful as a local CLI in the public
[`ayni-oss/ayni`](https://github.com/ayni-oss/ayni) repository.

## Target product

The project foundation provides named projects with shared, editable repository
membership, project-owned tasks targeting one selected repository, archive
history, and admission checks. Projects is the landing page; Configuration owns
connector setup. [Decision 0011](docs/decisions/0011-project-centered-ui-and-onboarding.md)
defines the updated onboarding and UI direction. Connector management, project readiness, internal environment
production, and internal candidate assessment remain follow-up work tracked by
[the product epic](https://github.com/ayni-oss/orchestrator/issues/90).

Connectors manage access. Projects own repository readiness and environments.
Tasks develop and verify work. Both runtime profiles share admission, attempts,
candidates, cancellation, evidence binding, publication and reconciliation.
Ayni Managed adds certified environments and Ayni assessment policy; Custom
Runtime supplies a supported container without mandatory Ayni metadata.

A task freezes repository/source, profile revision, verified OCI digest and
execution limits. Ayni runs during development as feedback. A separate internal
Job verifies each final candidate commit before publication, with at most three
repair cycles. Drafts preserve valid work with its actual assessment outcome.
Evidence stays outside candidate-writable storage and binds the precise commit
and execution. GitHub Actions are not the target assessment authority.

## Documentation

| Document | Purpose |
| --- | --- |
| [Connectors, projects and internal assessments](docs/decisions/0009-connectors-projects-and-internal-assessments.md) | Accepted product journey, ownership, credential and assessment boundaries |
| [Clean-v1 runtime-profile decision](docs/decisions/0007-clean-v1-runtime-profiles.md) | Normative names, ownership, identity, lifecycle, trust, recovery, and issue map |
| [Architecture](docs/architecture.md) | Interface ownership, shared orchestration, runtime profiles, and implementation boundaries |
| [Interface boundaries](docs/decisions/0010-orchestration-interface-boundaries.md) | Dependency direction and incremental application extraction |
| [Protocol contract](proto/README.md) | Target clean-v1 gRPC resources and compatibility reset |
| [Local development](docs/local-development.md) | Target Docker Desktop workflow and validation boundary |
| [Operator UI and Kubernetes onboarding](docs/operator-onboarding.md) | Existing App, setup PR, signed environment, request and draft PR flow |
| [Operations](docs/operations.md) | Target readiness, recovery, retention, and evidence operations |
| [Roadmap](ROADMAP.md) | Linked delivery issues and follow-ups |
| [Local Helm deployment decision](docs/decisions/0006-local-helm-chart.md) | Existing local chart/deployment boundary; worker details are historical until revised |
| [Ayni 0.14 integration contract](docs/ayni-0.14-integration.md) | Released launcher, certification, and assessment-evidence boundaries |
| [Managed runtime execution](docs/managed-runtime-execution.md) | Helm-managed ServiceAccounts, registry/build sources, coding, and publication |
| [Managed PR assessments](docs/managed-pr-assessments.md) | Trusted Actions collection, immutable assessments, summaries, and readiness policy |
| [Custom-client gRPC access](docs/custom-client-access.md) | Private endpoint boundary; not public hosting authorization |

## Planning and delivery

Requirements and delivery status live in GitHub rather than duplicate planning
documents:

- [Ayni Orchestrator project](https://github.com/orgs/ayni-oss/projects/2)
- [Orchestrator issues](https://github.com/ayni-oss/orchestrator/issues)
- [Completed milestones](https://github.com/ayni-oss/orchestrator/milestones?state=closed)

Repository development rules and the private boundary with Ayni are recorded in
[AGENTS.md](AGENTS.md). Public Orchestrator announcements, documentation,
releases, packages, images, or links from Ayni require an explicit publication
decision.
