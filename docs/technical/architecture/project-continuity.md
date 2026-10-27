# Factual project continuity

> UI status: the local product supports configuration and basic workspace/project
> navigation; see [integration connections](../architecture/integration-connections.md).
> Conversation/task browser steps below describe the retired UI and remain
> historical. Their acceptance profiles require replacement coverage. The API
> gateway itself serves no UI assets.

Fresh conversations can read task lifecycle facts and accepted outcomes from their own project through `read_project_status`.
The tool uses the existing eligibility-time `ProjectStatusSnapshot` in the private-v1 conversation command.
Shared facts preserve task evidence without importing another conversation's working context.

## Authority and scope

PostgreSQL selects facts from admitted tasks, prepared attempts, accepted run state, managed-quality results and publication records.
Rejected or fenced worker reports cannot contribute accepted outcomes.
The command's workspace and project binding determine scope; model arguments cannot select another project.

| Included | Meaning |
| --- | --- |
| Project objective and registration status | Retained project configuration; registration does not assert objective completion. |
| Task and attempt identities | References to admitted work and its latest prepared attempt, when present. |
| Coding lifecycle and accepted result | Current run state and independently retained coding outcome, separate from managed quality. |
| Quality outcome and evidence identity | Retained validation state and evidence, when available. |
| Publication state and retained draft reference | Publication lifecycle and previously retained draft URL, when available; the URL does not assert that the PR is still open. |
| Selection counts and incompleteness | A bounded selection cannot claim to contain the complete task history. |

Facts exclude conversation transcripts, task titles, plan text, prompts, model reasoning, patches and raw tool output.
Other conversations retain their own isolated context and tool ledgers.
Reading facts grants no approval, continuation, publication or merge authority.

## Frozen selection

One PostgreSQL statement reads the objective, project resource version, task count and bounded facts from a consistent database view.
Each admitted task contributes its latest prepared attempt, selected by descending creation time and attempt ID.
An unprepared task has coding state `awaiting_execution` and an empty attempt ID; a prepared attempt without a run has state `prepared`.

Facts are ordered by attempt creation time descending, with unprepared tasks last, then task ID descending.
The snapshot returns at most 20 facts, including current lifecycle states as well as terminal outcomes.
Recovery facts retain `parent_attempt_id`, so a linked continuation does not erase the parent identity.

| Snapshot field | Contract |
| --- | --- |
| `tasks` | Bounded `ProjectTaskFact` selection. |
| `total_tasks` | Optional count of all admitted tasks in the bound project, including unprepared tasks; absent on older snapshots. |
| `task_facts_available` | Tool-output flag derived from count presence. True means the task selection was captured, including a verified empty project. |
| `has_more_tasks` | True when the selection omits admitted tasks beyond the bound. |
| `task_id`, `attempt_id`, `parent_attempt_id` | Task, latest attempt and direct recovery-parent references. |
| `coding_state` | Retained run lifecycle or the explicit awaiting/prepared state. |
| `accepted_coding_state` | Empty until a canonical coding outcome is retained; then `completed`, `failed` or `uncertain`. |
| `provider_state` | Retained provider execution state, preserving uncertainty separately from cancellation or interruption. |
| `quality_state`, `quality_artifact_digest` | Retained managed-quality outcome and artifact identity, when available. |
| `publication_state`, `publication_id`, `pull_request_url` | Retained publication lifecycle, identity and draft reference, when available. |

The complete snapshot is retained in the exact command bytes alongside the conversation's frozen context.
Later coding, quality or publication transitions affect subsequent eligible turns, not an existing snapshot.
Command replay and repeated tool invocation use retained inputs rather than recomputing current status.

The existing 32 KiB tool-output limit and turn-wide input allowance still apply.
An over-limit read fails explicitly; it does not silently drop facts or invent a summary.

Older retained commands replay unchanged with unknown task availability.
Their tool output has `task_facts_available: false` and `total_tasks: null`.
Current snapshots with a verified empty project have `task_facts_available: true` and `total_tasks: 0`.

## Outcome interpretation

Coding, quality and publication are separate stages.
Successful coding does not imply passing quality, and a retained draft does not establish merge or deployment.
An interrupted attempt or uncertain provider outcome remains explicit even when saved work exists.
Cancellation can change the run lifecycle while preserving an earlier accepted coding result.
Retained provider uncertainty remains visible even when the run is cancelled.

Project status remains `registered`.
This factual projection invokes no model and does not assess whether the objective is complete.
Outcome assessments use a separate, explicitly authorized conversation turn.
They do not rewrite this factual projection or establish project completion.
The [M7 delivery plan](../research/project-workflow-m7-delivery-plan.md) records the integration acceptance scope.

## Authorized outcome assessment

In the contributor console, select a conversation and open **Task outcome assessments**.
Review the workspace model and input/output limits, choose an allowance of 1–10 assessments, and confirm consent.
Each assessment can incur a provider call within that frozen configuration's limits.
Saving 0 stops future admission; the ordinary conversation controls cancel already admitted turns.
`AuthorizeOutcomeAssessments` retains exact scope, configuration revision and allowance with idempotent replay.
`GetOutcomeAssessmentStatus` exposes the retained allowance and admitted count after reload.
Changing workspace configuration pauses the old authorization until the operator reviews and grants a new one.

The maintenance sweep coalesces accepted outcomes into at most one turn while the originating conversation is idle.
It freezes a bounded selection of the 16 most recent accepted task outcomes, containing stage states and exact accepted evidence digests.
The snapshot excludes transcript text, patches and private reasoning.
Accepted outcome timestamps survive replay pruning, so a new outcome on an older attempt enters the selection.
Progress without a changed accepted outcome schedules no model call.
Append-only scan cursors prevent unchanged conversations from starving later work in bounded sweeps.
One snapshot is admitted once per authorization, including after cancellation or restart.
The ordinary retained turn owns request identity, token reservation, result replay and provider uncertainty; an ambiguous request is never retried as another assessment.

Assessments can use the existing structured proposal tool to retain a reviewable follow-up plan.
The originating conversation remains the proposal owner, and the normal exact-version approval boundary owns task admission.
A changed task evidence snapshot rejects stale assessment proposals before plan materialization.
The stale interpretation never enters shared project facts.
Recovered work and model recovery advice retain their separate task-recovery approval boundary.

## Acceptance boundary

- A fresh conversation receives lifecycle facts and accepted outcomes only from its bound workspace and project.
- Another conversation's messages, titles, plans and reasoning remain absent from project facts and conversation context.
- New task outcomes cannot rewrite an already retained project snapshot or tool result.
- Recovery selection, ordering, counts and the 20-fact bound remain explicit and deterministic.
- Invalid scope, unsupported states and inconsistent selection metadata fail contract validation.
- Duplicate report delivery, control-plane restart and retained command replay preserve the same snapshot identities and bytes.
- Deployed acceptance exercises the factual path without paid-provider calls or external publication authority.

The delivery PR owns executed repository and Docker Desktop Kubernetes evidence.
The [conversation execution contract](conversation-execution.md) owns tool budgets and frozen-context semantics.
