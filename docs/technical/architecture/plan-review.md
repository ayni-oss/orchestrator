# Durable plan review

> UI status: the local product supports configuration and basic workspace/project
> navigation; see [integration connections](../architecture/integration-connections.md).
> Conversation/task browser steps below describe the retired UI and remain
> historical. Their acceptance profiles require replacement coverage. The API
> gateway itself serves no UI assets.

The first M3 slice retains operator-authored plans in their originating conversation.
It implements review and revision from the [M3 contract](plans-and-approval.md).
[Structured agent proposals](agent-proposals.md) extend this review path; [approval and task admission](task-admission.md) bind explicit authorization to an exact reviewed version.

## Content and identity

`PlanContent` contains title, objective, included/excluded scope, constraints,
unresolved questions and ordered proposed task specifications.
Each task specifies a version-local key, objective, scope, acceptance criteria,
declared repository/source, execution limits and unresolved inputs.
Repository references are declarations, not verified evidence or execution permission.

| Record | Retained authority |
| --- | --- |
| Plan | Server-assigned ID, workspace, project, originating conversation, current version and optional same-conversation follow-up link. |
| Version | Server-assigned ID and ordinal, predecessor, immutable content bytes/digest, title, `local-operator` author and timestamp. |
| Write receipt | Conversation/method/key, exact request digest, command ID, version ID, conversation revision and resume cursor. |

Composite foreign keys bind the complete parent chain, predecessor and current
version to the same plan. The runtime can update only the current-version pointer;
it cannot update version content or delete plans, versions or receipts.
Task admission copies the immutable origin and exact version reference. Approved plans reject additional revisions.

## Canonical content and bounds

The persistence model serializes typed `PlanContent` using compact `serde_json`
with declaration-order fields, snake-case names and all fields present.
No maps or unknown fields are stored; strings and array order are preserved without
Unicode normalization. `content_digest` is `sha256:` plus lowercase SHA-256 of those exact bytes.

Request digests use the existing length-delimited request digest function over
method, owner IDs, plan ID, expected version, follow-up ID and canonical bytes.
The create-only fields are empty in revise commands; server-assigned IDs do not enter request content.

| Bound | Limit |
| --- | --- |
| Canonical stored version | 32,768 UTF-8 JSON bytes; reject rather than truncate. |
| Plans per conversation | 32, including follow-up plans. |
| Versions per plan | 64; exhausted capacity still permits exact receipt replay. |
| Proposed tasks per version | 32; an incomplete draft may have none. |
| Title | 1–256 UTF-8 bytes; nonblank and no surrounding whitespace. |
| Objective, scope, excluded scope | 4,096 UTF-8 bytes each; objective and included scope are nonblank. |
| Constraints, questions, acceptance criteria, unresolved inputs | At most 16 nonblank items per list; 2,048 UTF-8 bytes per item. |
| Task key | 1–64 ASCII letters, digits, underscore or hyphen; unique within a version. |
| Declared repository/source | 1,024/256 UTF-8 bytes; empty means unspecified. |
| Execution limits | 2,048 UTF-8 bytes of declared constraints; empty means unspecified, never unlimited authority. |
| Version-history page | Default 10, maximum 20 summary records; exclusive descending ordinal cursor. |
| List/snapshot | At most 32 plan heads; full content is fetched separately. |

All strings reject NUL. Accepted versions and receipts remain retained; there is
no deletion endpoint or automatic compaction. These per-conversation limits do
not establish overall production capacity, which remains M8 acceptance.

## Commands and observations

| Canonical v1 method | Behavior |
| --- | --- |
| `CreatePlan` | Owner scope, idempotency key, content and optional same-conversation `follows_plan_id`; returns initial version and receipt. |
| `RevisePlan` | Owner scope, key, plan ID, expected version ID and complete replacement content; returns appended version and receipt. |
| `GetPlanVersion` | Full owner scope and plan ID; empty version ID selects current content in one query. |
| `ListPlans` | Bounded current heads within one complete conversation scope. |
| `ListPlanVersions` | Immutable summary history within one plan; zero ordinal starts at newest, zero next cursor marks completion. |

Writes lock the owning conversation before checking receipts, capacity and the
expected head. They atomically retain the version, head pointer, receipt,
conversation revision, activity and `PLAN_VERSION_CREATED` watch update.
There is no broker command or coding dispatch.

Exact retries return the original version and receipt even after later revisions.
Changed content under the same key returns `IDEMPOTENCY_CONFLICT`.
Concurrent different-key edits serialize; a stale expected head returns
`ABORTED` with `PLAN_VERSION_CONFLICT` and changes nothing.

`GetConversationSnapshot` includes plan heads from its repeatable-read boundary.
Watch updates carry plan/version references without embedding content in activity.
Version history has independent immutable pagination; it is not inserted into
the chat transcript or another conversation's model context.

## Console and recovery

Select a plan to inspect its exact version, origin, digest, scope and proposed
tasks. Choose an older version from history to compare it with the current head.
**Edit current version** copies a reviewed head into a separate local editor;
saving appends a version rather than modifying the reviewed record.

Local drafts persist in tab-local session storage under their full owner scope.
Watch refresh and reconnect update plan heads without overwriting the reviewed
version or unsaved draft. A stale edit requires explicit current-version review;
the console never rebases or resubmits it automatically.

Pending writes use the console's existing exact-key retry panel across reloads.
PostgreSQL owns accepted content; browser storage is only a local editing/retry
convenience and can be lost if cleared. Plan writes do not authorize paid calls,
repository access or execution. Explicit approval is a separate exact-version command.

## Validation

The cumulative PostgreSQL gate includes `tests/plans.rs`: restart/replay, scope,
concurrent edits, immutable history, transaction rollback, bounds and runtime grants.
The API/gateway integration covers all five methods and typed conflicts.
The cumulative browser scenario includes lost-response retry, structured editing,
two-tab conflicts, draft preservation, history, escaping and scope rejection.

See [local development](../operations/local-development.md) for deployed checks
and [the delivery plan](../research/project-workflow-m3-delivery-plan.md) for later slices.
