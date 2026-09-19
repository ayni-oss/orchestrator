# Ayni Orchestrator

Private product and technical exploration of an orchestration layer for agentic
software development, built around Ayni's environment and quality contracts.

The repository remains `ayni-operator` until its GitHub rename is completed.
Product-facing names use **Ayni Orchestrator**.

**Status: private M2 durable orchestration in progress.** This repository contains
the Rust workspace, the M1 managed-check worker, and a locally deployable control
plane with persistent task and attempt state. Managed-execution and orchestration
completion evidence is still in progress; it does not yet contain an agent runtime
or a supported integration. The name is provisional; architecture proposals
remain subject to implementation evidence. No delivery date or public launch is
committed by these documents.

## Intent

Keep connected repositories ready for development. Turn a request into a
reviewable change using prepared environments, project context, development
services, and reproducible quality evidence. Support projects spanning several
repositories and agents people already use, including Cursor.

Ayni remains an independently useful local product. This repository explores the
additional lifecycle and coordination capabilities needed at project and
organization scale.

## Reading guide

| Document | Purpose |
| --- | --- |
| [MVP scope and experience](docs/mvp.md) | Initial audience, request-to-PR journey, scope boundaries, and completion requirements |
| [MVP validation plan](docs/mvp-validation.md) | Engineering gates, ten-task pilot, measurements, and investment decision |
| [Delivery milestones](docs/milestones.md) | M0-M5, one PR per milestone, deployment gates, and completion evidence |
| [Local deployment decision](docs/decisions/0001-local-kubernetes-deployment.md) | Docker Desktop Kubernetes, Kustomize, and the local deployment workflow |
| [Product name and control-plane model](docs/decisions/0002-product-name-and-control-plane-model.md) | Ayni Orchestrator, its user-facing configuration model, and the controller/CRD boundary |
| [Durable local orchestration](docs/decisions/0003-durable-local-orchestration.md) | M2 SQLite/PVC state, Job transport, idempotency, and recovery boundaries |
| [Product intent](docs/product-intent.md) | Problem, audience, experience, boundaries, and business hypothesis |
| [Architecture options](docs/architecture.md) | Domain model, responsibilities, topology options, and access diagram |
| [Environment lifecycle](docs/environment-lifecycle.md) | Onboarding, prepared environments, candidate changes, and development services |
| [Multi-repository work and evidence](docs/multi-repository.md) | Agent discovery, concurrent work, automatic recording, and integration acceptance |
| [Agents, integrations, and positioning](docs/integrations-and-positioning.md) | Cursor comparison and integration, Rig, providers, skills, tools, and MCP |
| [Broader validation backlog](docs/validation.md) | Longer-term experiments and unresolved choices beyond the MVP |
| [First iteration investigation](docs/first-iteration.md) | Rust feasibility, proposed execution milestone, and managed-worker options |

Development rules, including the private boundary with Ayni, are recorded in
[AGENTS.md](AGENTS.md).

## Local M0 development

See [local development](docs/local-development.md) for the pinned-toolchain
checks and the Docker Desktop Kubernetes build, deploy, health-check, and
scoped-cleanup workflow.

## How to read proposals

- **Intent** records the direction expressed during product exploration.
- **MVP scope** records the agreed initial product target; the MVP documents take
  precedence over broader exploration when deciding what belongs in this pilot.
- **Existing Ayni capability** describes the separate Ayni project at the source
  revision recorded below.
- **Proposal** describes a possible implementation to test.
- **Open question** identifies a choice that has not been made.

Examples of resource names, tool calls, states, and configuration concepts are
illustrative. They are not a Kubernetes custom-resource schema or a public API.

## Publication boundary

This material is private planning. The remote was verified as **PRIVATE** on
2026-09-15 before the initial documentation push. Keep repository visibility
private. Public announcements, public documentation, releases, publishing
workflows, and links from the public Ayni repository require a separate explicit
publication decision. A Markdown label does not enforce confidentiality;
repository access controls do.

An eventual open-source project and optional SaaS are intentions, not current
availability. The repository is licensed under Apache-2.0; packaging and
commercial boundaries remain undecided.

## Research baseline

The primary source of intent is the product exploration on 2026-09-15. Technical
claims about Ayni were checked against revision
`f4bae6ef087a7d98ccbcef1bfc71bdaec585b8ee` of
[`ayni-oss/ayni`](https://github.com/ayni-oss/ayni).
External product and library references are linked in the relevant documents;
recheck their capabilities before implementation or public claims.
