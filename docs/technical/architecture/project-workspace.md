# Project workspace

The Svelte workspace is the first slice of the [product roadmap](../../../ROADMAP.md#first-product-workspace-slice).
It provides workspace/project navigation and conversation chat over the existing
canonical private-v1 API. The contributor console remains available for advanced
operator journeys until equivalent product acceptance passes.

## Transport and authority

`dev/ctl web` builds `apps/web` and passes its build directory to `workflow-console`.
The gateway loads at most 16 MiB of regular HTML, JavaScript, CSS and SVG assets
into memory. Nested directories are bounded; symlinks and unsupported file types
are rejected. Exact asset lookup prevents arbitrary filesystem reads or history
fallback for unknown paths. Missing builds return an explicit prerequisite error.

`/workspace/` serves the product UI; `/` serves the console. The existing host,
origin, mutation-header, request/watch bounds and content-security policy apply to
both. Both bind only to loopback and forward to the pod-local canonical API.
The product UI receives no native gRPC runtime, broker/database credential or
Kubernetes capability. Build checks reject server imports in the browser output.

## Scoped browser state

Workspace, project and conversation discovery use bounded keyset pages. Restoring
a saved selection loads further pages until its resource is found or exhausted.
Parent validation precedes child discovery. Scope generations fence late catalog,
history, provider and watch results, including navigation back to a previous scope.
The console handoff carries bounded parent identifiers and reuses canonical discovery.

Each turn mutation freezes content, scope and a random command key before sending.
Tab-local storage retains one interrupted command across reloads. Definitive
non-admission clears it; ambiguous outcomes retain it and block new mutations.
The original command is inspectable and an explicit retry sends the same bytes.
Storage failure retains an in-memory retry with a warning; clearing storage can
lose the retry handle and must not become an implicit new-key resubmission.

Selection and pending commands have separate product-workspace storage keys from
the console. Transcripts and watch cursors are reconstructed rather than persisted.
Unsent drafts reset across conversation changes; they cannot appear in another
conversation. Page reload starts a cursorless watch and receives an atomic snapshot.

## Observation and retained evidence

SSE frames adapt `WatchConversation`. Updates refresh the canonical projection;
the browser advances its replay cursor only after applying it successfully.
Cursor expiry starts a fresh atomic snapshot. Bounded frame processing reconnects
from the last applied cursor when overwhelmed. Revision comparisons use integers
without losing precision above JavaScript's exact number range.

The inspector displays current activity, message/turn counts, recent updates in
this view, retained provider calls and usage, workflow identities and the latest
command receipt. Its update list is bounded and is not a full durable audit export.
Non-optional conversation counts use ProtoJSON zero defaults; omitted optional
provider usage remains unknown. No monetary cost, task success rate, model round
count or production capacity is inferred.

Closing a tab or pausing observation does not cancel accepted work. Cancellation
uses the exact turn command and does not assert remote revocation or a refund.
Uncertain provider outcomes point to the retained console recovery controls.
Content is rendered as text, including prompts, activity details and history.

## Acceptance boundaries

Unit tests cover retained command recovery, definitive versus ambiguous rejection,
stale scope/history/catalog results, keyset restoration, exact revisions, watch
cursor recovery and storage failure. Rust tests verify asset lookup, build
prerequisites, symlink rejection and inherited gateway security.

Browser-fixture acceptance covers creation, chat, reload, concurrent tabs,
disconnect/reconnect, accepted-response loss, exact retry, capacity rejection,
isolation, older history, content escaping, console scope, themes, mobile layout
and content-security policy. CI runs that profile against the built assets.

Run the same browser journey with `WEB_URL` against an owned Docker Desktop
namespace for real PostgreSQL/JetStream and isolated-session evidence. It uses the
scripted provider and retains its accepted data. Fixture results do not establish
pod execution or live-provider behavior. The [workspace guide](../../../apps/web/README.md)
contains commands; PRs record actual execution results and blockers.
