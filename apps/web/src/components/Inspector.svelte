<script>
  import { count, elapsed, readable } from '../api.mjs';
  import { hasNewActivity } from '../timeline.mjs';
  import { consoleLink } from '../api.mjs';
  let { view, now, client } = $props();
  const activity = $derived(view.snapshot?.currentActivity);
</script>
<section class="inspector" aria-label="Execution activity">
  <div class="inspector-title"><h2>Activity & evidence</h2><span class="badge">RETAINED</span></div>
  <p class="muted">Accepted workflow facts for this conversation.</p>
  {#if activity}
    <div class="current-stage"><span class="eyebrow">Current stage</span><h3>{readable(activity.stage)}</h3><p>{readable(activity.presentationState)}{activity.waitingOn && activity.waitingOn !== 'WAITING_ON_UNSPECIFIED' ? ` · ${readable(activity.waitingOn)}` : ''}</p><p>{readable(activity.safeReason)}</p>{#if activity.safeDetail}<p class="muted">{activity.safeDetail}</p>{/if}<small>{elapsed(activity.acceptedAt, now)} since this observation</small></div>
  {/if}
  <dl class="fact-grid"><div><dt>Messages</dt><dd>{view.snapshot ? count(view.snapshot.historyCounts?.messages || '0') : 'Not loaded'}</dd></div><div><dt>Turns</dt><dd>{view.snapshot ? count(view.snapshot.historyCounts?.turns || '0') : 'Not loaded'}</dd></div></dl>
  <h3>Execution timeline</h3>
  <p class="muted">Accepted transitions, newest first. Times record acceptance, not the exact start or duration of worker stages.</p>
  {#if hasNewActivity(view.snapshot, view.timeline)}<p class="notice" role="status">New activity is available.</p>{/if}
  <button class="text-button" onclick={() => client.timeline()} disabled={view.timeline.busy}>{view.timeline.busy ? 'Loading timeline…' : 'Refresh timeline'}</button>
  {#if view.timeline.error}<p class="error" role="alert">Timeline unavailable: {view.timeline.error}. Refresh to retry.</p>{/if}
  <ol class="timeline" aria-label="Execution timeline">
    {#each view.timeline.entries as entry (entry.activity.id)}
      {@const item = entry.activity}
      <li><strong>{readable(item.stage)} · {readable(item.presentationState)}</strong>
        <p>{readable(item.safeReason) || readable(entry.kind)}</p>
        {#if item.waitingOn && item.waitingOn !== 'WAITING_ON_UNSPECIFIED'}<p>Waiting on {readable(item.waitingOn)}</p>{/if}
        {#if item.safeDetail}<p class="muted">{item.safeDetail}</p>{/if}
        <small>{item.acceptedAt ? new Date(item.acceptedAt).toLocaleString() : 'Time not recorded'} · revision {item.conversationRevision}</small>
        <details><summary>Transition evidence</summary><dl class="identities"><dt>Activity</dt><dd>{item.id}</dd>{#if item.turnId}<dt>Turn</dt><dd>{item.turnId}</dd>{/if}<dt>Event</dt><dd>{entry.kind}</dd>{#each entry.affectedResourceIds || [] as id}<dt>Related resource</dt><dd>{id}</dd>{/each}</dl><a href={consoleLink(view.scope)}>Inspect plans, tasks & recovery ↗</a></details>
      </li>
    {:else}<li class="muted">{view.timeline.busy ? 'Loading retained transitions…' : 'No retained transitions in this snapshot.'}</li>{/each}
  </ol>
  {#if view.timeline.nextBeforeRevision !== '0'}<button class="text-button" onclick={() => client.timeline(true)} disabled={view.timeline.busy}>Load older activity</button>{/if}
  {#if view.timeline.retainedFromRevision !== '0'}<p class="muted">Retained history starts at revision {view.timeline.retainedFromRevision}. History pruned before timeline installation is unavailable.</p>{/if}
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
