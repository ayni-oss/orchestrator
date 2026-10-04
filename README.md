# Ayni Orchestrator

Ayni Orchestrator is a private development-workflow control plane that produces
reviewable repository changes on controlled infrastructure. Its pre-public v1
is being replaced by one shared foundation with **Ayni Managed** and **Custom
Runtime** runtime profiles. Ayni Managed is the first complete profile; Custom
Runtime is a named later follow-up. The product journey is **Connectors → Projects → Tasks**. The normative
product design is [Decision 0009](docs/decisions/0009-connectors-projects-and-internal-assessments.md),
which retains the shared runtime foundations from Decision 0007.

The checked-in pilot API, SQLite state, local preparation archives, and
validation lifecycle are retired design history. They are replaced directly,
without a migration, compatibility API, or legacy-read path by delivery issues
[#53–#68](https://github.com/ayni-oss/orchestrator/issues/51). This repository
remains private; Ayni is independently useful as a local CLI in the public
[`ayni-oss/ayni`](https://github.com/ayni-oss/ayni) repository.

## Target product

A project is a named workspace with one repository in this release. Tasks belong
to the project and freeze its repository revision, profile revision, and verified
OCI digest. Archiving retains history and prevents new tasks and retries.
Repository connections and runtime profiles remain independently reconciled
implementation resources beneath projects.

The accepted architecture adds encrypted dynamic VCS connectors, internally built
certified environments, and independent exact-commit Ayni assessments inside the
Orchestrator deployment. Ayni configuration and locks remain repository-owned
prerequisites. Registry/signing credentials are existing Kubernetes Secret
references; coding workloads never receive them. Delivery is tracked by
[the product epic](https://github.com/ayni-oss/orchestrator/issues/90).

## Documentation

| Document | Purpose |
| --- | --- |
| [Connectors, projects and internal assessments](docs/decisions/0009-connectors-projects-and-internal-assessments.md) | Product hierarchy, ownership, credential and assessment decisions |
| [Clean-v1 runtime-profile decision](docs/decisions/0007-clean-v1-runtime-profiles.md) | Normative names, ownership, identity, lifecycle, trust, recovery, and issue map |
| [Architecture](docs/architecture.md) | Target shared/profile component and resource model |
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
