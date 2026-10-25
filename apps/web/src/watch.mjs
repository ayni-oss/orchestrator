// Advance the replay cursor only after the corresponding snapshot is applied.
export function watchConversation({ scope, request, apply, activity, status, error, EventSource, schedule = setTimeout, cancel = clearTimeout }) {
  let source, timer, cursor = '', disposed = false, queued = 0, chain = Promise.resolve();
  const close = () => { source?.close(); source = null; cancel(timer); };
  function reconnect(reset = false) {
    close();
    if (reset) cursor = '';
    if (!disposed) { status('Reconnecting'); timer = schedule(connect, 1000); }
  }
  function connect() {
    if (disposed) return;
    close();
    status('Connecting');
    const query = new URLSearchParams(scope);
    if (cursor) query.set('afterCursor', cursor);
    const current = new EventSource(`/watch?${query}`);
    source = current;
    current.onopen = () => { if (source === current) status('Live'); };
    current.onerror = () => { if (source === current) status('Reconnecting'); };
    current.addEventListener('frame', event => {
      if (source !== current || disposed) return;
      if (++queued > 64) { queued--; reconnect(); return; }
      chain = chain.then(async () => {
        if (source !== current || disposed) return;
        const frame = JSON.parse(event.data);
        if (frame.heartbeat) return;
        const snapshot = frame.snapshot || (frame.update && (await request('GetConversationSnapshot', scope)).snapshot);
        if (source !== current || disposed || !snapshot) return;
        if (snapshot.workspaceId !== scope.workspaceId || snapshot.projectId !== scope.projectId || snapshot.conversationId !== scope.conversationId) throw new Error('Conversation scope mismatch.');
        apply(snapshot);
        if (frame.update) activity(frame.update);
        cursor = frame.update?.cursor || snapshot.resumeCursor || '';
      }).catch(cause => {
        if (source === current && !disposed) { error(cause); reconnect(); }
      }).finally(() => { queued--; });
    });
    current.addEventListener('fault', event => {
      if (source !== current || disposed) return;
      try {
        const fault = JSON.parse(event.data);
        const expired = fault.reason === 'ERROR_REASON_WATCH_CURSOR_EXPIRED';
        if (!expired) error(new Error(fault.reason || fault.code || 'Watch interrupted.'));
        reconnect(expired);
      } catch (cause) { error(cause); reconnect(); }
    });
  }
  connect();
  return () => { disposed = true; close(); };
}
