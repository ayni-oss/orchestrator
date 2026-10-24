# Approval and task admission

An operator approves one exact immutable plan version from its originating
conversation. PostgreSQL retains one approval receipt and every reviewed task in
the same transaction. Tasks remain `awaiting_execution`; admission creates no
coding command, worktree, provider call or publication.

See the [binding plans contract](plans-and-approval.md), [plan content and
bounds](plan-review.md) and [M3 delivery plan](../research/project-workflow-m3-delivery-plan.md).

## Approval command

`ApprovePlan` requires the full workspace/project/conversation scope, plan ID,
expected current version ID, exact `content_digest`, and scoped idempotency key.
The trusted local operator is retained as `local-operator`; model proposals and
read tools have no approval authority.

Approval requires at least one task, acceptance criteria for every task and no
unresolved plan-level scope questions. Unresolved task execution inputs do not
block admission: they remain frozen and visible as execution blockers. Declared
repository/source references and execution limits do not establish verification,
runtime selection or authorized execution budget. Task dependencies name keys in
the exact reviewed task set; validation rejects missing, repeated, self and cyclic
edges. Preparation requires [explicit upstream selections](task-dependencies.md).

Approval and revision serialize on the owning conversation. Approval either
retains the reviewed current head or returns `PLAN_VERSION_CONFLICT`; it never
substitutes newer content. A wrong content digest returns `PLAN_DIGEST_MISMATCH`.
Unresolved scope returns `PLAN_UNRESOLVED_SCOPE`. Another key after approval
returns `PLAN_ALREADY_APPROVED`. Approved plans reject further revisions; new
work requires an explicitly linked follow-up plan.

## Atomic receipt and immutable origin

The transaction retains the approval, complete ordered task specifications,
conversation revision, activity and `PLAN_APPROVED` watch update together. A
failed transaction admits no partial task set. Unique plan approval and approved
version/task-key constraints prevent duplicate admission under competing keys.

The receipt retains command identity, request digest, exact reviewed version and
digest, operator, timestamp, task IDs and conversation revision/resume cursor.
Receipt lookup precedes mutable head/approval preconditions: the same key and
content recover the original identities after response loss or restart. Changed
content under the same key conflicts.

Each task retains workspace, project, originating conversation, plan, version,
content digest, approval ID, proposed-task key and frozen specification.
Composite constraints protect the ownership chain; runtime grants prohibit
rewriting or deleting retained origin. No execution authority derives from a
browser selection or model-supplied routing.

## Reads and console recovery

| Canonical v1 method | Result |
| --- | --- |
| `GetPlanApproval` | Optional retained approval plus the complete admitted task set for a fully scoped plan. |
| `ListTasks` | Conversation-scoped task page; exclusive task-ID cursor, default 20 and maximum 32 records. |
| `GetTask` | One admitted task under its full immutable parent scope. |

Plan heads in lists and conversation snapshots include `approval_id`. Watch
reconnect refreshes the authoritative snapshot and reconstructs admission without
issuing another command. Task reads do not import another conversation's private
content into model context.

The console shows the reviewed version/digest and requires explicit consent
before approval. New streamed heads preserve reviewed content and local drafts;
stale review disables approval until the operator opens and reviews the current
version. Opening another version resets consent. Approval receipt and task detail
show immutable origin, frozen specifications and execution blockers. An ambiguous
approval remains in the existing pending-command panel across reload and retries
with its original scope, version, digest and key.

## Validation and handoff

The cumulative storage gate tests exact retries, wrong scope/digest, unresolved
scope, competing approvals, revision races, complete admission, rollback and
retained-state recovery. The browser harness extends review acceptance with
explicit consent, stale review, two-tab approval conflict, response-loss/reload
retry, task pagination, scoped reads and immutable-origin display.

M4 must verify and freeze execution inputs under the task's retained authority.
M6 routes questions/failures to the originating conversation. M7 can use accepted
outcomes for factual project continuity. The [M3 acceptance matrix](../operations/plan-acceptance.md) defines cumulative
deployment, interruption and retained-state recovery gates for the fourth slice.
