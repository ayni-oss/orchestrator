<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/site/public/ayni-orchestrator-dark.svg">
  <img src="docs/site/public/ayni-orchestrator.svg" alt="Ayni Orchestrator logo" width="64" height="64">
</picture>

# Ayni Orchestrator

Ayni Orchestrator is building an Ayni-first software factory: project chat develops
reviewed plans, autonomous agents execute modular tasks, and results return as
reviewable pull requests. Tasks can run alone, in sequence or as a dependency DAG;
the plan chooses their PR layout. Humans retain merge authority.

This is the alpha target, not a claim that the complete journey is delivered.
The [roadmap](ROADMAP.md) defines executable milestones and acceptance; the
[alpha product contract](docs/technical/architecture/alpha-product.md) defines scope.

## Product responsibilities

Workspaces define available repositories and shared context, instructions, skills
and MCP capabilities. Projects select the repositories needed for a chunk of
changes. Project chat researches source, validates plans, follows progress and
resolves questions. Approved agents should inspect, edit, write tests, build and
repair within their scope and allowance, including dependency/environment changes.
Workspace repository scope and the full engineering loop remain planned.

Ayni owns tool integrations, exact locks, portable environments and code-quality
policy evaluation. Orchestrator owns planning, execution authority, isolation,
runtime publication, task coordination and retained delivery evidence. It consumes
Ayni contracts rather than duplicating adapter or lock semantics. Ayni is alpha's
first runtime path; custom containers remain future scope.

Passing managed Ayni quality is required for delivery. The requested behavior also
needs acceptance evidence; quality alone does not establish product success.
Execution, quality, acceptance and PR publication remain separate outcomes.

## Current delivery boundary

The sole [web application](apps/web/README.md) provides workspace/project creation,
named GitHub/GHCR/OpenAI configuration, repository discovery/catalog, direct project
associations and exact-commit static Ayni inspection. Workspace repository
availability is not yet enforced; project chat, plan review and complete task
browser journeys remain planned.

Runtime preparation implements bounded Kubernetes build, separate certification,
publication, independent content/managed checks, signing setup and recoverable
history. It requires an explicitly qualified profile, registry and trusted signer.
The default Docker Desktop profile remains disabled after failed network-policy
enforcement; a separate native-arm64 Cilium topology has Node component and
supervisor lifecycle evidence. See [runtime packaging](deploy/images/runtime-preparation/README.md)
for the exact tested envelope, released CLI prerequisites and live integration gaps.

Canonical v1 backend APIs implement durable conversations, isolated sessions,
immutable plan approval, frozen task attempts, managed quality, recovery,
dependency selections, draft publication and execution timelines. Planning's
built-in tools currently read instructions, project facts and conversation history;
coding currently uses a single bounded file-proposal pass. Task source/runtime
preparation still requires a trusted local helper.

Conversation handoffs use JetStream; coding tasks use HTTP callbacks and supervisor
polling. The [task event direction](docs/technical/architecture/task-events.md)
extends durable orchestration while preserving PostgreSQL authority, explicit
permits and stale-worker fencing. Event transport alone does not establish capacity.

The [foundation roadmap archive](docs/technical/history/backend-foundation-roadmap.md)
retains M1–M8 provenance and acceptance limits. Current-source browser qualification
must replace retired console coverage. Fixture/provider protocol tests do not
establish autonomous coding, live forge publication or production readiness.

## Local development and documentation

Use [local development](docs/technical/operations/local-development.md) for the
explicit cluster contexts, immutable deployment, live editing and retained-state
operations. `dev/ctl live` runs the sole frontend and backend live loop. Setup and
runtime qualification have documented prerequisites; the alpha journey must not
require per-task terminal source-pack preparation.

- [Contributor preview](docs/site/index.md) explains current local product availability.
- [Alpha workflow](docs/site/alpha.md) explains the intended workspace, project, task and PR experience.
- [Technical documentation](docs/technical/README.md) indexes current contracts and planned extensions.
- [Repository preparation](docs/technical/architecture/repository-preparation.md) defines source/runtime gates and Ayni boundaries.
- [Architecture](docs/technical/architecture/architecture.md) defines current ownership and durable execution.

See [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) for branch, review,
validation and publication rules. Interfaces remain v1 during alpha; old SQLite
workflow, BFF and retired frontend paths are not compatibility requirements.
