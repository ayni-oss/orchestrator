# Contributor conversation console

The local contributor console exercises the canonical workflow with the retained
scripted executor. It is maintained through M2, where real isolated sessions and
provider conversations will use the same surface. Shared configuration and
scripted session inspection/replacement use canonical v1 methods.

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
current configuration. Desired provider/model settings are visible while actual
execution remains scripted. Existing admitted turns keep their frozen inputs.

The conversation session panel lists scoped retained generations and exposes
idle replacement. A replacement command binds the exact expected generation.
The advisory `can_replace` flag enables the button after active turns and
outbox handoffs settle; the command rechecks this boundary transactionally.
Refresh the panel while it displays finishing accepted work. An ambiguous retry
reuses the original command.

The next eligible message wakes a new generation. Session controls and
configuration edits retain the gateway's existing authority and credential boundary.

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
