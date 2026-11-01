# Project workspace

Status: configuration, workspace/project creation and rename, shared repository
catalog/direct project associations, static inspection and gated runtime preparation
are available in the sole [web application](../../../apps/web/README.md). Project
chat, plan/task delivery screens and their replacement browser qualification remain
planned. See [integration connections](integration-connections.md) for current APIs.

## Alpha workspace and project experience

The [alpha product contract](alpha-product.md) defines the target hierarchy:
workspaces make repositories and shared capabilities available; a project selects
the subset needed for a chunk of changes. The catalog remains shared. Workspace
availability/project-subset enforcement are not implemented by current direct
catalog-to-project associations.

Project chat is the working surface for ongoing research, plan generation and
validation, progress, questions and results. Plans and task graphs remain
inspectable alongside chat. A project may span several repositories; each modular
task targets one repository and each PR belongs to one repository.

| Surface | Alpha user outcome |
| --- | --- |
| Workspace settings | Choose available catalog repositories, context, instructions, skills, MCP capabilities and shared defaults. |
| Project setup | Select the required repository subset and project context; inspect source/runtime readiness and blockers. |
| Project chat | Research selected source, retrieve earlier decisions, propose/review plans and respond to real execution blockers. |
| Plan and work graph | Review an exact version, standalone/sequential/DAG tasks, grants and chosen shared/separate/stacked PR layout. |
| Execution inspector | Follow tool rounds, build/tests, Ayni validation, dependency readiness, cancellation, repair and separate outcomes. |
| Delivery and feedback | Navigate task/commit/PR relationships and retained evidence; continue project discussion without granting merge authority. |

Keep the existing shell, themes and shared components. All slices extend this one
application and canonical APIs; no separate diagnostic product or historical BFF
is required. The gateway remains an allowlisted API adapter without UI assets.

## Browser acceptance

Every implemented journey needs reload/reconnect, exact command retry, stale-tab
conflicts and scope-switch coverage. Closing a browser cannot cancel accepted
work. Live output remains provisional until accepted; reads and previews cannot
grant execution or publication authority.

The [roadmap](../../../ROADMAP.md#delivery-sequence) owns delivery order. The
[repository delivery plan](../research/repository-runtime-workspace-delivery.md)
details source/runtime UI cases. Historical console procedures retain provenance
but do not qualify the current frontend.
