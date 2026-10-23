# Contributor conversation console

The local contributor console exercises the canonical workflow with retained
isolated sessions. Shared configuration, scripted or OpenAI execution, provider
usage, cancellation and session inspection/replacement use canonical v1 methods.

## Access and authority

`dev/ctl console` forwards the control-plane Pod's loopback gRPC listener onto a
random host loopback port and starts the Rust `workflow-console` binary. It serves
embedded HTML, CSS and JavaScript on `http://127.0.0.1:3000`. Both its listener and
upstream must be loopback IP addresses. Closing the command stops its port-forward;
a Pod restart requires rerunning the command.

The gateway owns no database connection, broker connection, execution state, or
credentials. An explicit method allowlist forwards ProtoJSON commands and queries
to canonical gRPC. Native Node gRPC bindings never enter browser bundles. Protobuf
64-bit fields remain decimal strings; timestamps and enum names use ProtoJSON.

Host and Origin checks, fetch-metadata checks, a custom mutation header, no CORS,
a restrictive content-security policy and `no-store` responses protect the local
browser boundary. This is the trusted contributor/operator profile, not a remote
identity or authorization system. Static assets render retained content as text.
At most 32 unary requests and eight watches run concurrently per gateway. Request
bodies are bounded at 64 KiB; canonical input and response bounds still apply.

## Browser transport decision

The console uses same-origin JSON POST requests for commands and queries, and SSE
for `WatchConversation`. Both SSE and browser-compatible gRPC can carry the needed
unary calls and server stream. SSE fits this small contributor client without a
browser RPC runtime or an additional protocol proxy; the adapter stays explicit
and uses generated Rust ProtoJSON bindings rather than inventing workflow models.

Each SSE frame wraps the canonical snapshot, update, or heartbeat. Snapshot/update
IDs are canonical cursors. `Last-Event-ID` takes precedence over the initial query
cursor on automatic reconnect. Heartbeats do not advance the cursor. Typed gRPC
errors, including `WATCH_CURSOR_EXPIRED`, become `fault` events; they never acquire
an SSE ID. Stream backpressure and slow-reader eviction remain owned by canonical
gRPC; the gateway has no unbounded event queue and drops upstream on disconnect.

The UI refreshes the authoritative snapshot when an update arrives, then advances
its in-memory replay cursor. A failed refresh closes the stream and retries from
the last successfully processed cursor. Older revisions cannot replace newer
projections. Expired cursors restart with an atomic snapshot. Page refresh starts
cursorless because a cursor alone is insufficient to rebuild browser state.

Alternatives and protocol references: [SSE and EventSource](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events)
and [gRPC-Web's supported transports](https://github.com/grpc/grpc-web#streaming-support).

## Shared configuration and sessions

The workspace configuration editor reads and updates the canonical immutable
configuration snapshot. Updates carry the displayed expected revision and the
same retained idempotency key on retry. A stale edit fails without changing the
current configuration. Provider/model settings apply at admission. Actual provider calls and reported
usage appear separately in the conversation panel. Existing admitted turns keep their frozen inputs.

The conversation session panel lists scoped retained generations and exposes
idle replacement. A replacement command binds the exact expected generation.
The advisory `can_replace` flag enables the button after active turns and
outbox handoffs settle; the command rechecks this boundary transactionally.
Refresh the panel while it displays finishing accepted work. An ambiguous retry
reuses the original command.

The next eligible message wakes a new generation. Session controls and
configuration edits retain the gateway's existing authority and credential boundary.

## Provider usage and cancellation

**Provider calls and usage** lists retained calls for the selected conversation.
Each entry shows its frozen provider/model, call and turn IDs, state, reserved
allowance and reported token usage. An omitted usage counter displays **Unknown**;
it never becomes zero. Currency cost is not inferred from token counts. The list
refreshes after workflow updates, on reconnect and with **Refresh provider calls**.
Assistant transcript labels do not imply a specific provider.

**Cancel turn** binds the exact active or queued turn and uses the same retained
command retry mechanism as message admission. Closing the watch or tab does not
cancel accepted work. Cancellation can stop work before sending; it cannot prove
that an in-flight foreground request was revoked or refunded. Results and usage
already retained remain factual evidence even when cancellation wins visible
terminal acceptance.

An uncertain call keeps its identity, reservation and recovery evidence. The
panel explains that it will not be retried automatically. Refresh, reconnect,
Pod replacement and cancellation cannot resolve uncertainty. The operator must
reconcile the exact call and its retained evidence before authorizing further
work. An explicit acknowledgement binds the call and request digest and records
that a charge may have occurred. After the uncertain report and outbox handoffs settle,
acknowledgement closes the held turn and retires its session generation. Queued
work can then enter a fresh generation. The acknowledgement preserves uncertain
evidence, unknown usage and the reservation; it does not resend the call.

Tool activity lists allowlisted tool names, invocation IDs, states and digests.
Arguments, read outputs, full prompts and model reasoning stay out of activity.

## Plan review and task admission

Plans retain immutable reviewed versions separately from local drafts. Streamed
heads never rewrite reviewed content or unsaved edits. Approval requires explicit
consent to the displayed version/digest and complete task set; unresolved scope
questions and missing acceptance criteria block it. Approved task detail exposes
immutable project/conversation origin and execution blockers while tasks await
execution capability. No coding work is dispatched by approval.

`ApprovePlan` uses the same retained exact-command retry panel as other commands.
Snapshot plan heads carry the approval identity; scoped approval/task reads
reconstruct the receipt and task set after reconnect without another mutation.
See [plan review](plan-review.md) and [task admission](task-admission.md).

## Task observation and publication controls

**Tasks** lists canonical admitted work in pages of 20, independent of the selected
plan. `GetTask`, `GetTaskExecution` and `GetTaskAttempt` reconstruct the full
origin, preparation, attempt and separate execution/quality/publication outcomes.
One selected task refreshes every five seconds in a visible tab, without overlapping
requests. Scope and selection generations fence late reads, including selection
changes during an in-flight request. Hidden tabs issue no periodic reads. Failed
reads clear stale detail and consent. The list refresh remains explicit so later
inserts cannot silently replace an operator's pagination boundary.

Accepted patches, quality artifacts and retained identities render as expandable
text. Target counts and exit codes preserve observed zero. Provider uncertainty,
failed quality, interrupted work and retained publication blockers have distinct
guidance. There is no repair or new-attempt control. Trusted source verification,
preparation and source-pack upload remain outside the browser allowlist; a task
selection download carries only its originating scope and task ID.

`ReviewTaskPublication` is a read-only local gateway composition of canonical task
and attempt reads. It uses the same preview builder as `workflow-publish-task`.
It owns no durable state, publication receipt or credential. The UI displays the
canonical title, body, repository, source, branch, author and evidence digests;
explicit consent authorizes `StartTaskPublication` with the reviewed attempt and
exact patch/quality digests. State refresh invalidates an ineligible review;
changing task or scope always clears it. PostgreSQL rechecks eligibility and
competing publication keys transactionally. `CancelTaskPublication` is offered
only before the once-only create permit, and canonical cancellation rechecks that
fence. Publication authority remains separate from publisher credentials.

Coding cancellation and publication commands share exact pending-command recovery
across reloads. An accepted receipt refreshes only the task it belongs to. Creating
or uncertain publication cannot authorize a second create request. Retained PR
links must exactly match the recorded public GitHub repository and decimal PR
number. The console reports the last recorded external state without implying a
fresh GitHub read. See [draft publication](task-publication.md).

## Task feedback and project attention

`ListTaskFeedback` reads retained execution and quality problems for either the
selected conversation or its project. The console displays system notices
separately from assistant messages and opens each notice's originating
conversation/task for evidence inspection. Reads use bounded pages and discard
late responses after selection changes; reconnect and reload restore canonical
feedback rather than trusting a browser copy.

Notices remain open in this slice. The interface exposes no answer, dismiss,
resolve or continue command, and reading a notice cannot authorize task execution.
Publication reconciliation retains its existing task views. See the
[task feedback contract](task-feedback.md) for scope, deduplication and retained-state rules.

## Discovery, history and retries

`ListWorkspaces`, `ListProjects` and `ListConversations` are canonical read-only
RPCs. Parent scopes are validated before child discovery. Pages use exclusive ID
keysets, default to 50 records, and accept at most 100. These are live catalogs,
not frozen snapshots: refresh to find concurrent inserts. Selection remains in
tab-local session storage; lists are fetched again after reload. Child lists do
not expose resources owned by other parents.

Conversation history continues to use the canonical stable history snapshot and
bounded reverse pagination. The browser pins one history boundary while loading
older pages and does not mix it with later live snapshot boundaries. Typed expiry
resets that local pagination boundary on the next attempt.

Before a mutation, the browser retains its exact method, scope, content and random
idempotency key in tab-local session storage. Only one unresolved mutation is
allowed per tab. Network failures retain that command and expose an explicit retry;
retry and reload never generate a new key for it. Successful receipts clear it. Definitive validation, missing-parent, idempotency-conflict and canonical
`RESOURCE_EXHAUSTED` with `ERROR_REASON_BOUND_EXCEEDED` rejections clear the local
retry so the user can correct or resubmit the request. An untyped capacity error
or ambiguous transport failure retains it.
No automatic new-key resubmission follows an ambiguous outcome. Browser storage
is a convenience for retries, not authority; clearing it can lose the local retry
handle. Transcripts and replay cursors are not persisted in browser storage.

See [local development](../operations/local-development.md) for operation and
[the roadmap](../../../ROADMAP.md#m1m2-delivery-scopes) for delivery scopes.
