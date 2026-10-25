# Execution timeline

The workspace inspector reads accepted transitions through
`ProjectConversationService.ListExecutionTimeline`. It explains retained workflow
history alongside the live current activity. This is the first bounded slice of
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
and offers retry. The inspector renders content as text and exposes turn/activity
identities, related resources and a scoped console handoff for further inspection.

Accepted timestamps describe when the database recorded an observation. They do
not establish exact stage starts or durations. Stage names retain their current
meaning; task execution, quality and publication remain separate evidence in the
console. Provider rounds, detailed worker progress, incremental assistant text,
cross-service trace coverage and task-specific drill-down remain later slices.
The timeline read emits a bounded workflow diagnostic span; traces are never the
authority for retained history.

## Verification

The PostgreSQL gate exercises runtime-role inserts through the trigger, exact
command replay, stable pagination during new admission, restart, watch compaction,
scope and bounds rejection, forbidden deletion and disabled-trigger readiness.
Browser tests cover reload, cross-tab new-activity notices, refresh, scope isolation
and the same gateway method used in deployment. Unit tests cover stale page/scope
results, exact large revision values, deduplication and failed-read recovery.
Browser fixtures alone do not establish deployed PostgreSQL or worker behavior.
