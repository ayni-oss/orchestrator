<script>
  import { elapsed, readable } from '../api.mjs';
  import { progressRound } from '../execution.mjs';
  import { turnStatus } from '../workspace.mjs';
  let { view, client, now } = $props();
  let draft = $state('');
  let transcript = $state();
  let following = $state(true);
  const messages = $derived.by(() => {
    const history = view.history.filter(record => record.kind === 'CONVERSATION_HISTORY_RECORD_KIND_MESSAGE').map(record => ({ id: record.id, sequence: record.sequence, content: record.contentOrOutcome, acceptedAt: record.acceptedAt, role: record.roleOrState === 'CONVERSATION_HISTORY_ROLE_OR_STATE_USER_MESSAGE' ? 'MESSAGE_ROLE_USER' : 'MESSAGE_ROLE_ASSISTANT' }));
    return [...new Map([...history, ...(view.snapshot?.recentMessages || [])].map(message => [message.id, message])).values()].sort((a, b) => BigInt(a.sequence || 0) < BigInt(b.sequence || 0) ? -1 : 1);
  });
  $effect(() => {
    if (messages.length && following && transcript) transcript.scrollTop = transcript.scrollHeight;
  });
  const provisional = $derived(view.progress.filter(item => item.provisionalText));
  $effect(() => {
    if (provisional.length && following && transcript) transcript.scrollTop = transcript.scrollHeight;
  });
  async function send(event) {
    event.preventDefault();
    const content = draft, origin = view.scope.conversationId;
    if (await client.submit('SendMessage', { ...view.scope, content }) && view.scope.conversationId === origin && draft === content) draft = '';
  }
</script>
<section class="conversation-panel" aria-label="Conversation">
  <div class="conversation-status" role="status"><span class:working={!!view.snapshot?.nonterminalTurns?.length} class="status-dot"></span><span>{turnStatus(view.snapshot)}</span></div>
  <div class="transcript" aria-label="Messages" bind:this={transcript} onscroll={event => { const element = event.currentTarget; following = element.scrollHeight - element.scrollTop - element.clientHeight < 80; }}>
    {#if view.historyMore}<button class="history-button" disabled={view.historyBusy} onclick={() => client.older()}>{view.historyBusy ? 'Loading history…' : 'Load earlier history'}</button>{/if}
    {#each messages as message (message.id)}
      <article class="message" class:from-user={message.role === 'MESSAGE_ROLE_USER'}>
        <div class="message-avatar" aria-hidden="true">{message.role === 'MESSAGE_ROLE_USER' ? 'Y' : 'a'}</div>
        <div class="message-body"><div class="message-meta"><strong>{message.role === 'MESSAGE_ROLE_USER' ? 'You' : 'Assistant'}</strong><time datetime={message.acceptedAt}>{message.acceptedAt ? new Date(message.acceptedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''}</time></div><p>{message.content}</p></div>
      </article>
    {:else}
      <div class="empty-conversation"><span class="empty-symbol" aria-hidden="true">✳</span><h2>Start with a question.</h2><p>Explore an idea, clarify the scope or talk through your next change.</p><p class="muted">Each conversation has its own isolated agent context.</p></div>
    {/each}
    {#each provisional as progress (progress.turnId)}
      <article class="message provisional-message" aria-label="Provisional assistant output">
        <div class="message-avatar" aria-hidden="true">a</div><div class="message-body"><div class="message-meta"><strong>Assistant</strong><span class="badge">PROVISIONAL</span></div><p>{progress.provisionalText}</p><small>{progressRound(progress)} · Partial output awaiting acceptance{view.connection.startsWith('Paused') ? ' · updates paused' : ''}</small></div>
      </article>
    {/each}
    {#if view.progressError && view.snapshot?.nonterminalTurns?.length}<p class="notice" role="status">Live output is temporarily unavailable. Accepted work continues; the next observation retries.</p>{/if}
    {#each view.snapshot?.nonterminalTurns || [] as turn (turn.id)}
      <div class="turn-row"><div><strong>{readable(turn.executionState)}</strong><span class="muted"> · {elapsed(turn.acceptedAt, now)} elapsed</span><small>{view.snapshot.currentActivity?.safeReason || 'Your accepted work continues independently of this tab.'}</small></div><button disabled={!!view.pending || view.sending || view.busy} onclick={() => client.submit('CancelTurn', { ...view.scope, turnId: turn.id })}>Cancel turn</button></div>
    {/each}
    {#each view.snapshot?.recentTerminalTurns?.filter(turn => turn.safeFailureReason) || [] as turn (turn.id)}<p class="error">Turn failed: {turn.safeFailureReason}</p>{/each}
  </div>
  {#if !following}<button class="jump-latest text-button" onclick={() => following = true}>Jump to latest messages ↓</button>{/if}
  <form class="composer" onsubmit={send}>
    <label for="message-content" class="sr-only">Message</label><textarea id="message-content" bind:value={draft} rows="3" maxlength="16384" required placeholder="What would you like to work on?" disabled={!view.snapshot || view.busy || view.sending || !!view.pending}></textarea>
    <div class="composer-footer"><span>Accepted work continues when you close this tab.</span><button class="primary" disabled={!view.snapshot || !draft.trim() || view.busy || view.sending || !!view.pending}>{view.sending ? 'Sending…' : 'Send message'} <span aria-hidden="true">↑</span></button></div>
  </form>
</section>
