# Task feedback and attention

> UI status: the local product supports configuration and basic workspace/project
> navigation; see [integration connections](../architecture/integration-connections.md).
> Conversation/task browser steps below describe the retired UI and remain
> historical. Their acceptance profiles require replacement coverage. The API
> gateway itself serves no UI assets.

Task feedback brings accepted execution and quality problems back to their originating conversation.
Project attention lists the same retained records across that project's conversations.
These are factual system notices, separate from assistant messages and model reasoning.

## Authority and retention

PostgreSQL retains feedback in the transaction that accepts the actionable task outcome.
Each record binds the immutable workspace, project, conversation, task and attempt origin.
The first actionable record per attempt/category is immutable; exact report retries and later stop reconciliation do not duplicate it.
A rejected or fenced worker report has no authority to create feedback.

| Category | Actionable outcomes |
| --- | --- |
| `execution` | Failed or interrupted coding, exceeded execution deadline, or provider uncertainty. |
| `quality` | Failed, blocked or interrupted managed quality. |

Intentional cancellation alone does not create attention.
Possible provider charges remain actionable even when cancellation wins the visible task outcome.
The record retains an allowlisted safe reason; its summary never includes provider output, credentials, prompts or source content.
Users inspect accepted patches and quality artifacts through the existing scoped task views.

Publication uncertainty can resolve through existing reconciliation.
It stays in publication views until a later attention lifecycle can represent that resolution without stale open items.
Preparation blockers also retain their existing task views.

## Canonical read

`ProjectConversationService.ListTaskFeedback` uses the existing private-v1 contract.

| Request field | Meaning |
| --- | --- |
| `workspace_id`, `project_id` | Required parent scope, validated even when no feedback exists. |
| `conversation_id` | Exact originating conversation; empty selects project attention. |
| `after_id` | Exclusive ascending ID-keyset boundary. |
| `limit` | Default 20, maximum 32. |

Each returned `TaskFeedback` contains its ID, full origin, task/attempt IDs, category, state, safe reason, summary and creation timestamp.
The response carries `next_after_id` when another page is available.
These are live bounded reads, not a frozen project snapshot.
Selecting another project or conversation cannot expose the previous selection's late response.

## Conversation interface

The console reads feedback for the selected conversation and project attention through its allowlisted local gateway.
Opening an attention item selects its originating conversation and task for evidence inspection.
Reload and reconnect reconstruct notices from canonical retained reads rather than browser storage.
The gateway owns no attention state or new execution authority.

Feedback projects `open` until an exact recovery confirmation exists for its parent attempt,
then `resolved`. The confirmation is the authority; reads never change the immutable notice.
Resolved means that the recovery decision was handled, not that coding or quality succeeded.
The projection includes the directly linked recovery attempt and its current coding and quality
states. A failed continuation produces its own notice; cancellation does not reopen the parent's
authorization or erase possible charges. Further recoveries preserve each parent/child link.

The project attention panel filters resolved decisions from each bounded page; users can
load more pages even when a page contains only resolved items. Conversation history keeps
both open and resolved notices, links to previous evidence, and an **Inspect continued outcome**
action. Refresh, focus and reload read canonical state. [Recovery review](task-recovery.md)
records proposals and operator question/answer records without resolving attention or dispatching
work. Notices and operator answers are not inserted into provider context automatically.
Structured worker questions remain outside the current coding protocol.

## Acceptance boundary

- Accepted execution/quality problems and their notices commit together or both roll back.
- Duplicate report delivery and supervisor stop replay preserve one immutable notice per attempt/category.
- Wrong-project/conversation reads fail scope validation; rejected stale reports leave no notice.
- Control-plane and PostgreSQL restarts preserve notice identity, origin and safe reason.
- Conversation/project navigation and reconnect cannot render stale notices from another scope.
- Reading or opening attention cannot send another provider request or start task execution.

The delivery PR records executed repository and Docker Desktop Kubernetes evidence.
The [task acceptance profile](../operations/task-acceptance.md) retains its coding, managed-quality and publication fixture boundaries.
