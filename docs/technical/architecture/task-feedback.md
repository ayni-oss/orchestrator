# Task feedback and attention

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

Every record is `open` in this slice.
Feedback has no dismiss, resolve, retry or continue mutation, and notices are not inserted into provider context automatically.
[Recovery review](task-recovery.md) adds separate immutable proposals and operator question/answer records; those records leave attention open and dispatch no work. Structured worker questions and attention resolution belong to later [M6 slices](../research/project-workflow-m6-delivery-plan.md).

## Acceptance boundary

- Accepted execution/quality problems and their notices commit together or both roll back.
- Duplicate report delivery and supervisor stop replay preserve one immutable notice per attempt/category.
- Wrong-project/conversation reads fail scope validation; rejected stale reports leave no notice.
- Control-plane and PostgreSQL restarts preserve notice identity, origin and safe reason.
- Conversation/project navigation and reconnect cannot render stale notices from another scope.
- Reading or opening attention cannot send another provider request or start task execution.

The delivery PR records executed repository and Docker Desktop Kubernetes evidence.
The [task acceptance profile](../operations/task-acceptance.md) retains its coding, managed-quality and publication fixture boundaries.
