# Task recovery review

Recovery review lets users inspect a proposed next action without starting it.
The originating task retains immutable operator proposals, questions and answers.
[Task feedback](task-feedback.md) remains open throughout this slice.
The [M6 delivery plan](../research/project-workflow-m6-delivery-plan.md) owns later confirmation and execution.

## Proposal and saved evidence

A local operator selects an execution or quality feedback item and records an explanation,
proposed action (`continue`, `repair_quality`, or `replace_attempt`), additional seconds
and additional tokens. Both allowances must be positive: at most 86,400 seconds and
1,000,000 tokens. They describe proposed limits; recording them allocates no budget.
The selected attempt must be terminal and the feedback must belong to its exact origin.
These are operator-authored review records, not model-generated recommendations.

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
confirmed continuation. A replacement action only proposes a future explicit link;
it creates no attempt and changes no approved inputs or downstream task selections.

## Questions and answers

A local operator can record a question against an existing task run and append one
answer. The answer links the exact question under the same attempt. The question and
answer are separate immutable records, with `local-operator` provenance. Concurrent
answers serialize: only the first is retained. A different answer requires a new
question. Answering supplies direction and grants no continuation authority.

The coding worker protocol still cannot ask structured questions. Worker-originated
questions, model-generated proposals and the complete conversation journey remain
later M6 work. Operator text is displayed as text and is not inserted into provider context.

## Canonical private-v1 API

`ProjectConversationService` adds two methods:

| Method | Contract |
| --- | --- |
| `RecordTaskRecovery` | Requires workspace, project, conversation, task, attempt and command key, plus exactly one proposal, question or answer. Returns the original immutable record on an exact retry. Changed content under that key conflicts. |
| `ListTaskRecovery` | Requires the same exact origin. Returns ascending ID-keyset pages with `after_id`, default limit 20, maximum 32 and `next_after_id`. Missing and foreign attempts fail even for empty history. |

Each record retains its full origin, ID, author, content digest and timestamp.
The content digest hashes the server's stored JSON bytes, not a caller serialization.
Migration 21 installs the append-only review table; the runtime role cannot update or
delete records. Writes serialize on the task run, check retries before inspecting
current evidence, and commit a complete record atomically. Restart and response loss
preserve the original record and digest. Proposal revisions create new records.

The local console exposes recovery review beside task evidence reached from conversation
feedback. It uses the existing retained pending-command path for response-loss retries.
Navigation fences late reads and preserves the command's original scope.
History refresh and reload read canonical state. No browser record is execution authority.

## Authority and acceptance

Recording, listing and answering do not dispatch workers, send provider requests,
consume the proposed allowance, publish changes, resolve attention or change task state.
There is no confirmation command in this slice. Future confirmation must bind an exact
proposal ID/digest and revalidate saved evidence, current state and approved inputs.
M5 dependency scheduling remains a prerequisite for dependency-aware continuation.

The PostgreSQL gate covers exact retry and changed-content conflict, concurrent question
and answer writes, scoped reads, bounded history, fresh-store replay and runtime immutability.
Cumulative deployed coding/quality acceptance retains proposal, question and answer bytes
and verifies them after control-plane and PostgreSQL restarts, alongside unchanged attempt
and feedback identities. Browser tests cover review, reload, lost responses and navigation.
Each delivery PR records executed evidence and any blocked checks. Coding fixtures do not
prove live provider continuation, GitHub recovery or the later M6 confirmation loop.
