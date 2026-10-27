# Task recovery review

> UI status: the local product supports configuration and basic workspace/project
> navigation; see [integration connections](../architecture/integration-connections.md).
> Conversation/task browser steps below describe the retired UI and remain
> historical. Their acceptance profiles require replacement coverage. The API
> gateway itself serves no UI assets.

Recovery review lets users inspect a proposed next action without starting it.
The originating task retains immutable operator and worker proposals, questions and answers.
[Task feedback](task-feedback.md) remains open until exact confirmation handles its recovery decision.
Exact confirmation now authorizes one linked execution segment for the existing independent-task runtime.
The [M6 delivery plan](../research/project-workflow-m6-delivery-plan.md) owns the complete recovery journey.

## Proposal and saved evidence

A local operator selects an execution or quality feedback item and records an explanation,
proposed action (`continue`, `repair_quality`, or `replace_attempt`), additional seconds
and additional tokens. Both allowances must be positive: at most 86,400 seconds and
1,000,000 tokens. They describe proposed limits; recording them allocates no budget. Only confirmation authorizes those additional limits.
The selected attempt must be terminal and the feedback must belong to its exact origin.
Operator-authored proposals and accepted worker recommendations use the same explicit confirmation boundary.

The server freezes the safe reason, attempt state and generation, source commit,
runtime image, configuration revision, accepted patch and outcome digests, and
quality state and evidence digest. Callers cannot supply those evidence fields.
The proposal also explains whether an earlier provider request may have been charged.
That uncertainty is retained; a new proposal cannot acknowledge or retire it.
No monetary estimate is shown because this slice has no reliable pricing basis.

Users inspect the accepted patch and quality artifacts through `GetTaskAttempt`.
A proposal references those artifacts by digest; it does not duplicate or overwrite them.
Missing accepted work is explicit. Unreported worker files, provider checkpoints and
unaccepted progress are unavailable evidence. Presence of a patch does not establish
that it can be safely reused: reconstruction and validation remain prerequisites of
confirmed continuation. A `replace_attempt` proposal explicitly selects the original frozen source without seeding saved changes.
Confirming it links a new attempt while retaining the parent patch. It cannot substitute
a new source commit, runtime, configuration or reviewed goal. Material changes still require a new reviewed plan.

## Questions and answers

A local operator can record a question against an existing task run and append one
answer. The answer links the exact question under the same attempt. The question and
answer are separate immutable records, with `local-operator` provenance. Concurrent
answers serialize: only the first is retained. A different answer requires a new
question. Answering supplies direction and grants no continuation authority.

The coding worker can end a segment with a structured blocking question and bounded recovery recommendation. Canonical answers are frozen into an explicitly confirmed continuation, as described below.

## Canonical private-v1 API

`ProjectConversationService` provides these methods:

| Method | Contract |
| --- | --- |
| `RecordTaskRecovery` | Requires workspace, project, conversation, task, attempt and command key, plus exactly one proposal, question or answer. Returns the original immutable record on an exact retry. Changed content under that key conflicts. |
| `ConfirmTaskRecovery` | Requires exact task origin, proposal ID/digest, command key and explicit acknowledgement when the proposal reports provider uncertainty. Returns one durable confirmation and linked attempt. |
| `ListTaskRecovery` | Requires the same exact origin. Returns ascending ID-keyset pages with `after_id`, default limit 20, maximum 32 and `next_after_id`. Includes canonical answers for questions on the page, even when answers occur later, and the parent confirmation on every page. Missing and foreign attempts fail even for empty history. |

Each record retains its full origin, ID, author, content digest and timestamp.
The content digest hashes the server's stored JSON bytes, not a caller serialization.
History includes confirmations separately from immutable record content; confirming never changes a proposal digest.
Migration 21 installs the append-only review table; the runtime role cannot update or
delete records. Writes serialize on the task run, check retries before inspecting
current evidence, and commit a complete record atomically. Restart and response loss
preserve the original record and digest. Proposal revisions create new records.

The local console exposes recovery review beside task evidence reached from conversation
feedback. It uses the existing retained pending-command path for response-loss retries.
Navigation fences late reads and preserves the command's original scope.
History refresh and reload read canonical state. No browser record is execution authority.

## Confirmed continuation

Confirmation serializes with worker ownership and publication. It rechecks terminal
state/generation, source/runtime/configuration and accepted patch/outcome/quality
identities against the exact proposal. Stale evidence, a superseded attempt, unresolved
approved inputs or an existing publication rejects the new confirmation. A parent
consumed by recovery cannot gain new publication authority. Existing exact receipts
remain replayable before mutable eligibility checks.

Two tabs confirming the same proposal converge on one receipt and one attempt, even
with different command keys. Each key also retains its own exact replay binding.
A different proposal for the same parent cannot create a second continuation.
Confirmation, immutable attempt inputs, queue admission, latest execution projection,
observation and receipt commit together or all roll back. Migration 22 adds append-only
confirmation authorities, permits linked attempts and retains one pending run per task.
The bounded queue admits isolated independent attempts under the configured concurrency limit.

Each continuation uses a fresh attempt, Pod/PVC and provider ledger. This is a new
bounded execution segment linked to the existing task; no uncertain request or consumed
sending permit is reused. Source pack, commit, lock, runtime, configuration and approved
task stay frozen. Additional time/tokens are the proposal's limits for this segment;
possible charges from the earlier request remain unknown and separately acknowledged.
No monetary estimate is invented. An identical confirmation retry never admits another call.

For `continue` and `repair_quality`, the new initializer reconstructs the original
snapshot and restores the accepted patch before coding can obtain a provider permit.
Patch digest, lock and allowed file paths are verified; unavailable or invalid saved
inputs fail the new attempt before a provider request. Added files belong in the coding
context. The next accepted result is a cumulative patch against the original commit.
Prior accepted changes remain available as the frozen input patch even if a continuation
fails before accepting new output. The parent patch and quality evidence are never rewritten.
The bounded coding request includes the reviewed explanation and retained quality
artifact; oversized context fails explicitly rather than truncating evidence.

`replace_attempt` starts from the same original source without applying the parent's
patch. The confirmation interface states that choice explicitly. A quality repair is
one reviewed coding/quality pass with a fresh allowance, not an automatic repair loop.
New failure or uncertainty requires a new proposal and confirmation.

The console requires explicit consent to the exact proposal and, when applicable,
a separate possible-charge acknowledgement. Pending commands retain the original
proposal/digest and scope through response loss and reload. A confirmation links both
attempts; users can inspect previous evidence and the continued attempt. A selected
attempt stays selected through refresh/reload, so a new attempt cannot replace evidence
being reviewed. `GetTaskExecution` projects the latest linked attempt; original preparation
and start receipts still replay their original results.

## Dependency boundary

[Task dependencies](task-dependencies.md) freeze exact upstream task, attempt and
published commit selections before execution. Confirmed recovery copies those
selections and the integrated source commit with its retained inputs. A newer
upstream result cannot silently reselect, rebase or replace approved inputs.
Material source changes require new reviewed work.

## Authority and acceptance

Recording, listing and answering grant no execution or publication authority. Confirmation
alone admits a linked attempt under the additional allowance and resolves the parent attention
decision. Conversation notices retain the original problem and link to the continued outcome,
with coding and quality shown separately. The console hides competing consent controls after
another proposal consumes the parent, reconciles answers after a conflict, and refreshes canonical
history on task refresh and tab focus. Unchanged reads preserve drafts and consent; navigation
and changed history require a fresh review. Lost responses retain the exact pending command
through reload. Browser state grants no execution authority. Worker questions and canonical answers enter provider context only through a confirmed continuation.

The PostgreSQL gate covers exact and conflicting retries, confirmation races, stale
evidence, missing charge acknowledgement, rollback after dispatch insertion, frozen
inputs, old-worker rejection, latest-attempt reads and preserved parent evidence.
Source tests prove new saved files reach context and corrupt patch identities fail closed.
Cumulative deployed coding/quality acceptance confirms a proposal concurrently and executes
its linked attempt, then compares parent/child evidence and confirmation receipts through
control-plane and PostgreSQL restarts. A separate real browser profile exercises consent,
accepted-response loss, reload and navigation between linked attempts.
Each delivery PR records executed evidence and any blocked checks. Deterministic coding
fixtures do not prove live-provider recovery, GitHub recovery or complete M6 acceptance.

## Worker questions and model recommendations

The coding provider can end its bounded segment with `request_task_recovery` rather
than file changes. The response carries one blocking question, explanation,
proposed action and positive additional time/token limits. It grants no execution
or publication authority. The existing provider sending permit, model identity,
usage bounds and accepted worker generation bind this output exactly as coding output.

Once the supervisor confirms the worker has stopped, the terminal transition
atomically retains an originating-conversation question and model-authored recovery
proposal against the accepted failure evidence. Reserved `worker:` command keys
supply agent provenance and deduplicate stop/report replay; operator writes cannot
use that namespace. Historical records remain immutable.

The user records one canonical answer and reviews the exact proposal before
confirmation. An unanswered worker question blocks continuation. Confirmation
freezes at most 32 answered questions into the new segment, along with existing
saved-work, quality evidence and selected upstream inputs. The coding request
includes those exact question/answer pairs. Answering alone starts no worker or
provider call; new or changed source inputs still require reviewed work.
