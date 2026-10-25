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
Updates in the inspector cover the current browser view; they are not a complete audit export.

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

Incremental model text, complete execution traces, task success analytics, event
triggers and trusted browser-driven source preparation remain planned in the
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
