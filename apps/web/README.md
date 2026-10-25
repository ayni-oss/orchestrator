# Project workspace

This Svelte browser application provides the first product workspace over the
canonical private-v1 workflow. It restores the former web application's visual
shell and mark while using the current Rust gateway and retained workflow APIs.

## Run locally

From the repository root, after a locked dependency install and Docker Desktop deployment:

```sh
pnpm install --frozen-lockfile
dev/ctl deploy
dev/ctl web
```

Open `http://127.0.0.1:3000/workspace/`. `dev/ctl web 3001` selects another port.
The command builds static assets and starts the loopback gateway with a local
port-forward. Restart the command after control-plane pod replacement.
The gateway loads a bounded asset snapshot at startup; rebuilding requires restarting it.

Create a workspace, project and conversation, then send a message. Project
navigation, retained transcript, stable older history, cancellation and current
activity use existing canonical commands and resumable SSE observation.
Provider usage preserves unknown counters and uncertainty. Message counts and
turn counts are retained conversation facts, not task success rates or round metrics.
The inspector restores accepted execution transitions across reloads. Load older
activity for stable history pages, or refresh when new activity is available.
History pruned before timeline installation is unavailable; accepted timestamps
do not measure exact worker-stage duration. See the
[timeline contract](../../docs/technical/architecture/execution-timeline.md).

The contributor console remains at `/` for configuration, plans, task evidence,
publication, assessments and recovery. Workspace links carry the exact selected
scope. Neither surface grants access beyond the existing trusted local operator
profile. Remote identity and multi-user authorization are outside this slice.

## Recovery and boundaries

- An interrupted mutation retains its original command, content, scope and key in tab-local session storage. Retry reuses them exactly, including after reload.
- Definitive non-admission clears the key; an ambiguous transport or capacity failure keeps it. New mutations wait until that outcome is confirmed.
- Scope changes fence late reads and close the former watch. Unsent drafts reset when the selected conversation changes.
- Reload reconstructs canonical state from an atomic snapshot. Reconnect resumes after successfully applied updates; cursor expiry restarts from a snapshot.
- A disconnected watch does not cancel work. Cancellation is an explicit command and does not promise remote provider cancellation or a refund.
- Browser storage failure leaves an in-memory retry and an explicit warning to keep the tab open. Clearing storage can lose an interrupted command's local retry handle.
- Content is rendered as text. Native gRPC, broker, database and Kubernetes credentials remain outside the browser.

## Inspect execution

The conversation shows live assistant text in a separate provisional message.
It is replaced by the retained assistant message only after terminal acceptance.
Reload or reconnect reads the latest bounded progress snapshot; it does not retry
execution or a provider request. Pausing updates pauses observation while accepted
work continues. Failed progress reads retain the previous display, explain the
interruption and retry on the next observation.

Open **Turns & current progress** for turn identities, queue/model/tool states,
rounds, generation, provider call and elapsed time since the recorded stage
observation. Open **Task execution & delivery** to select an admitted task and
inspect its frozen source, attempt, accepted patch, managed quality artifacts,
publication and recovery questions separately. Active turns and attempts offer
explicit cancellation. Source preparation, recovery confirmation and publication
review continue through the scoped contributor console.

Task intervals use database acceptance boundaries and include waiting and handoff
time. Historical boundaries missing before timeline installation are shown as
not recorded. A quality failure preserves the coding outcome; a publication
failure does not rerun coding. Diagnostic trace identities correlate inspection
with the configured trace collector and do not replace retained evidence.

Task success analytics, event triggers and trusted browser-driven source
preparation remain planned in the
[product roadmap](../../ROADMAP.md#product-delivery-sequence).

## Validation

```sh
pnpm web:check
pnpm web:test
pnpm web:build
pnpm exec playwright install chromium
pnpm web:acceptance
```

The default browser profile uses a loopback fixture and measures UI behavior only.
It covers creation, chat, reload, two tabs, reconnect, lost responses, exact retry,
capacity rejection, scope isolation, history, escaping, console handoff, themes,
mobile layout and the production content-security policy.

For actual PostgreSQL/JetStream and isolated pod evidence, run against the gateway
connected to a separate owned Docker Desktop deployment:

```sh
WEB_URL=http://127.0.0.1:3000/workspace/ \
  WEB_EVIDENCE_DIRECTORY=/absolute/new/evidence pnpm web:acceptance
```

The deployed profile uses the default scripted provider and retains its created
workspaces, conversations and worker evidence. It does not qualify live providers.
Run the existing console acceptance separately; this slice does not retire it.

To include a real prepared task in deployed browser acceptance, provide its scope
and optionally an exact task/attempt selection. Omitting `taskId` selects the
first admitted task returned by the canonical API:

```sh
WEB_URL=http://127.0.0.1:3000/workspace/ \
  WEB_EVIDENCE_DIRECTORY=/absolute/new/evidence \
  WEB_TASK_SCOPE='{"workspaceId":"workspace-id","projectId":"project-id","conversationId":"conversation-id"}' \
  pnpm web:acceptance
```

This opens a separate tab and verifies canonical task/attempt evidence, independent
coding/quality/publication presentation and retained timing. Its report records
the exact task and attempt. The fixture profile separately exercises partial
output, lost progress reads, reload reconciliation and contract-shaped task
outcomes; those checks do not qualify a provider or managed task execution.
