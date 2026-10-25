<script>
  import { count, elapsed, readable } from '../api.mjs';
  let { view, now } = $props();
  const activity = $derived(view.snapshot?.currentActivity);
</script>
<section class="inspector" aria-label="Execution activity">
  <div class="inspector-title"><h2>Activity & evidence</h2><span class="badge">RETAINED</span></div>
  <p class="muted">Accepted workflow facts for this conversation.</p>
  {#if activity}
    <div class="current-stage"><span class="eyebrow">Current stage</span><h3>{readable(activity.stage)}</h3><p>{readable(activity.presentationState)}{activity.waitingOn && activity.waitingOn !== 'WAITING_ON_UNSPECIFIED' ? ` · ${readable(activity.waitingOn)}` : ''}</p><p>{readable(activity.safeReason)}</p>{#if activity.safeDetail}<p class="muted">{activity.safeDetail}</p>{/if}<small>{elapsed(activity.acceptedAt, now)} since this observation</small></div>
  {/if}
  <dl class="fact-grid"><div><dt>Messages</dt><dd>{view.snapshot ? count(view.snapshot.historyCounts?.messages || '0') : 'Not loaded'}</dd></div><div><dt>Turns</dt><dd>{view.snapshot ? count(view.snapshot.historyCounts?.turns || '0') : 'Not loaded'}</dd></div></dl>
  <h3>Updates in this view</h3>
  <ol class="timeline">{#each [...view.activities].reverse() as update (update.cursor)}<li><strong>{readable(update.presentationState)}</strong><p>{readable(update.safeReason) || 'Workflow state changed'}</p><small>{update.acceptedAt ? new Date(update.acceptedAt).toLocaleTimeString() : ''} · revision {update.conversationRevision}</small></li>{:else}<li class="muted">Live updates appear here. The current state and transcript are restored on reconnect.</li>{/each}</ol>
  <details><summary>Provider calls & usage</summary>
    {#if view.providerError}<p class="error">Usage is unavailable: {view.providerError}</p>{/if}
    {#each view.providerCalls as call (call.callId)}
      <article class="provider-call"><strong>{call.provider} / {call.model}</strong><p>{readable(call.state)}</p><dl><dt>Input tokens</dt><dd>{count(call.inputTokens)}</dd><dt>Output tokens</dt><dd>{count(call.outputTokens)}</dd><dt>Total tokens</dt><dd>{count(call.totalTokens)}</dd></dl>
      {#if call.uncertaintyAcknowledged}<p class="muted">Uncertain outcome acknowledged. A charge may have occurred; its evidence remains retained and the call will not be retried.</p>{:else if call.state === 'uncertain'}<p class="error">The provider outcome is unknown. This call will not be retried automatically. Reconcile it in the contributor console.</p>{/if}
      {#if call.cancellationRequested}<p class="muted">Cancellation requested. This does not confirm remote cancellation or a refund.</p>{/if}
      {#each call.toolInvocations || [] as tool (tool.invocationId)}<p class="tool-fact">{tool.name} · {tool.state}</p>{/each}
      <details><summary>Call identity</summary><code>{call.callId}</code></details>
      </article>
    {:else}<p class="muted">No retained provider calls in this view. Scripted execution does not imply provider usage.</p>{/each}
  </details>
  <details><summary>Workflow identities</summary><dl class="identities"><dt>Conversation</dt><dd>{view.scope.conversationId}</dd><dt>Revision</dt><dd>{view.snapshot?.conversationRevision || 'Not loaded'}</dd><dt>Last command receipt</dt><dd>{view.lastReceipt?.commandId || 'None in this tab'}</dd><dt>Observation cursor</dt><dd>{view.snapshot?.resumeCursor || 'Not loaded'}</dd></dl></details>
</section>
