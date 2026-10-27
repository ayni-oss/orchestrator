# Execution timeline

> UI status: the local product supports configuration and basic workspace/project
> navigation; see [integration connections](../architecture/integration-connections.md).
> Conversation/task browser steps below describe the retired UI and remain
> historical. Their acceptance profiles require replacement coverage. The API
> gateway itself serves no UI assets.

The workspace inspector reads accepted transitions through
`ProjectConversationService.ListExecutionTimeline`. It explains retained workflow
history alongside the live current activity. It is the retained-history boundary for
[observable execution](../../../ROADMAP.md#product-delivery-sequence).

## Retention and authority

PostgreSQL stores one immutable timeline entry per accepted conversation revision.
An insert trigger copies the activity, update kind and related resource identities
inside the transaction that writes a watch update. Rollback removes both records;
exact command/report retries use the existing deduplication boundaries. The runtime
can select and insert timeline rows but cannot update, delete or truncate them.
Readiness verifies the retention trigger and its definition in the catalog manifest.

Seven-day watch compaction does not remove timeline entries. Migration 28 seeds
history from existing paired activity/update rows; facts pruned before installation
are unavailable. `retained_from_revision` identifies the earliest available entry,
not a guarantee of complete history before installation. Timeline storage grows
with accepted transitions and must be included in database capacity and backup
planning. There is no automatic timeline retention deletion or audit export.

## Reads and browser behavior

Every read validates workspace, project and conversation ownership. Pages are
newest first, default to 50 entries and permit at most 100. A zero snapshot revision
selects the current conversation revision in a repeatable-read transaction.
Continuation requests retain that boundary and use an exclusive `before_revision`.
New writes cannot shift or duplicate older pages. Future and overflowing bounds,
or a continuation without a snapshot boundary, are rejected. Timeline activity
cursors are empty and cannot resume a live watch.

The browser loads history after its first authoritative snapshot. Current activity
continues updating live while the historical page remains frozen. A notice offers
refresh when the conversation advances; refresh starts a new boundary. Reload
reconstructs history from the API. Scope changes fence late reads, and a newer
refresh fences a pending older-page response. Read failure preserves loaded history
and offers retry. The inspector renders content as text and exposes turn/activity identities,
related resources, retained diagnostic trace IDs and scoped task evidence.
Conversation trace IDs come from the original admitted command receipt, survive
exact retry and queued FIFO promotion, and convey no authority. Older records
without retained context show no trace ID.

Accepted timestamps describe when the database recorded an observation. They do
not establish exact process or CPU time. Live progress, accepted turn timing and
task phase boundaries are presented separately in the inspector. Task coding,
quality and publication remain distinct outcomes, with evidence and explicit
next actions or scoped console recovery links. The original console remains
supported for approvals and operations whose web coverage is incomplete.

## Provisional progress

`ListExecutionProgress` returns full bounded snapshots for active conversation
turns. It validates the complete parent scope and session generation. The isolated
worker retains a latest snapshot and at most 64 stage markers per command under
an independent 8 MiB preview quota. The supervisor forwards authenticated progress
using the existing session-authority boundary. PostgreSQL accepts monotonic
sequences for the exact active binding; changed exact retries and stale generations
cannot replace newer observations. New stage observations enter the accepted
timeline with safe round/call/tool identities; token fragments do not become
individual domain events.

Each preview is at most 16 KiB. Model requests, public assistant text and tool
activity are distinct observations. Reasoning, tool arguments and raw tool output
are excluded. The browser fences stale responses by scope, request generation,
session generation and sequence. Reload and reconnect fetch the full preview;
observation loss does not initiate a new model call. The accepted terminal message
replaces provisional output rather than appending a second transcript message.
Canonical terminal acceptance or cancellation deletes the database preview; worker
result acknowledgement clears its retained preview. Preview quota failures do not
abort or retry paid execution. Existing provider uncertainty and cancellation
contracts continue to govern possible remote effects and charges.

OpenAI Responses and Anthropic Messages streaming adapters parse bounded SSE,
including split UTF-8 and mixed line endings. They require a valid terminal result
before accepting provider evidence. Partial or malformed streams retain uncertainty
through the existing sending checkpoint. A partial preview never establishes usage,
success or a completed response. Scripted progress is identified as scripted and
never implies model invocation or provider token usage.

## Verification

The PostgreSQL gate exercises runtime-role inserts through the trigger, exact
command replay, stable pagination during new admission, restart, watch compaction,
scope and bounds rejection, forbidden deletion and disabled-trigger readiness.
Browser tests cover reload, cross-tab new-activity notices, refresh, scope isolation
and the same gateway method used in deployment. Unit tests cover stale page/scope
results, exact large revision values, deduplication and failed-read recovery.
The [combined deployed profile](../operations/observable-execution-acceptance.md)
adds actual PostgreSQL/JetStream, isolated-session progress, task timing and
correlated traces. Browser fixtures alone do not establish deployed behavior.
