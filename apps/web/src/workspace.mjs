import { emptyTimeline, mergeTimeline } from './timeline.mjs';
import { request as gateway, readSaved, save, readable } from './api.mjs';
import { watchConversation } from './watch.mjs';

const selectionKey = 'ayni.workspace.selection';
const pendingKey = 'ayni.workspace.pending';
const knownCommands = new Set(['CreateWorkspace', 'CreateProject', 'CreateConversation', 'SendMessage', 'CancelTurn']);
const identity = /^[a-zA-Z0-9_-]{1,128}$/;
export function cleanScope(value = {}) {
  const scope = {};
  if (identity.test(value?.workspaceId || '')) scope.workspaceId = value.workspaceId;
  if (scope.workspaceId && identity.test(value?.projectId || '')) scope.projectId = value.projectId;
  if (scope.projectId && identity.test(value?.conversationId || '')) scope.conversationId = value.conversationId;
  return scope;
}
const sameScope = (a, b) => ['workspaceId', 'projectId', 'conversationId'].every(key => a[key] === b[key]);
export function isDefinitive(error) {
  return ['InvalidArgument', 'NotFound', 'AlreadyExists', 'Aborted', 'FailedPrecondition'].includes(error.code)
    || (error.code === 'ResourceExhausted' && error.reason === 'ERROR_REASON_BOUND_EXCEEDED');
}
export function turnStatus(snapshot) {
  if (!snapshot) return 'Choose a conversation';
  const activity = snapshot.currentActivity;
  if (!(snapshot.nonterminalTurns || []).length) return snapshot.recentTerminalTurns?.length ? 'All accepted turns have finished' : 'Ready for your first message';
  const waiting = { WAITING_ON_PREDECESSOR: 'Waiting for the previous turn', WAITING_ON_DELIVERY: 'Waiting for delivery', WAITING_ON_EXECUTOR: 'Waiting for an agent', WAITING_ON_PROVISIONING: 'Preparing an isolated session' };
  return waiting[activity?.waitingOn] || (activity?.stage ? readable(activity.stage) : 'Work accepted');
}

export function createWorkspace({ request = gateway, storage, EventSource, initialScope, schedule, cancel } = {}) {
  let state = {
    scope: cleanScope(initialScope || readSaved(storage, selectionKey, {})),
    workspaces: [], projects: [], conversations: [], pages: {}, busy: false, sending: false,
    snapshot: null, timeline: emptyTimeline(), activities: [], history: [], historyBusy: false, historyMore: false,
    providerCalls: [], providerError: '', connection: 'Choose a conversation', error: '',
    pending: readSaved(storage, pendingKey, null), lastReceipt: null, storageWarning: '',
  };
  if (!knownCommands.has(state.pending?.method) || !identity.test(state.pending?.body?.idempotencyKey || '')) state.pending = null;
  let epoch = 0, catalog = 0, providerQuery = 0, stopWatch, historyId = '', before = '0', disposed = false, timelineQuery = 0;
  const listeners = new Set();
  const emit = patch => {
    if (disposed) return;
    state = { ...state, ...patch };
    for (const listener of listeners) listener(state);
  };
  const showError = error => emit({ error: error.message || String(error) });
  const persist = (key, value) => {
    const stored = save(storage, key, value);
    if (!stored) emit({ storageWarning: 'Browser storage is unavailable. Keep this tab open until any pending command is confirmed.' });
    return stored;
  };
  function reset(scope) {
    stopWatch?.(); stopWatch = undefined; epoch++; providerQuery++; catalog++; timelineQuery++;
    historyId = ''; before = '0';
    persist(selectionKey, scope);
    emit({ scope, snapshot: null, timeline: emptyTimeline(), activities: [], history: [], historyBusy: false, historyMore: false,
      providerCalls: [], providerError: '', error: '', connection: scope.conversationId ? 'Connecting' : 'Choose a conversation' });
  }
  async function loadPage(kind, append = false) {
    const route = epoch, query = catalog, scope = { ...state.scope };
    const method = { workspaces: 'ListWorkspaces', projects: 'ListProjects', conversations: 'ListConversations' }[kind];
    const body = { limit: 50, afterId: append ? state.pages[kind] || '' : '' };
    if (kind !== 'workspaces') body.workspaceId = scope.workspaceId;
    if (kind === 'conversations') body.projectId = scope.projectId;
    const result = await request(method, body);
    if (route !== epoch || query !== catalog || disposed) return;
    const values = append ? [...state[kind], ...(result[kind] || [])] : result[kind] || [];
    emit({ [kind]: [...new Map(values.map(value => [value.id, value])).values()], pages: { ...state.pages, [kind]: result.nextAfterId || '' } });
  }
  async function findSelection(kind, id) {
    const route = epoch, version = catalog;
    await loadPage(kind);
    while (route === epoch && version === catalog && id && !state[kind].some(value => value.id === id) && state.pages[kind]) await loadPage(kind, true);
    return route !== epoch || version !== catalog || !id || state[kind].some(value => value.id === id);
  }
  async function refresh() {
    const version = ++catalog, route = epoch;
    emit({ busy: true, error: '' });
    try {
      if (!await findSelection('workspaces', state.scope.workspaceId)) throw new Error('This workspace is no longer available. Choose another workspace.');
      if (version !== catalog || route !== epoch) return;
      if (state.scope.workspaceId && !await findSelection('projects', state.scope.projectId)) throw new Error('This project is no longer available in the selected workspace.');
      if (version !== catalog || route !== epoch) return;
      if (state.scope.projectId && !await findSelection('conversations', state.scope.conversationId)) throw new Error('This conversation is no longer available in the selected project.');
    } catch (error) {
      if (version === catalog && route === epoch) { stopWatch?.(); emit({ snapshot: null, connection: 'Unavailable' }); showError(error); }
    } finally { if (version === catalog && route === epoch) emit({ busy: false }); }
  }
  async function providers() {
    const scope = { ...state.scope }, version = epoch, query = ++providerQuery;
    try {
      const result = await request('ListProviderCalls', scope);
      if (version === epoch && query === providerQuery) emit({ providerCalls: result.calls || [], providerError: '' });
    } catch (error) {
      if (version === epoch && query === providerQuery) emit({ providerCalls: [], providerError: error.message });
    }
  }
  async function timeline(append = false) {
    if (!state.scope.conversationId || (append && (state.timeline.busy || state.timeline.nextBeforeRevision === '0'))) return;
    const scope = { ...state.scope }, version = epoch, query = ++timelineQuery;
    const previous = state.timeline;
    emit({ timeline: { ...previous, busy: true, error: '' } });
    try {
      const page = await request('ListExecutionTimeline', { ...scope, limit: 50,
        snapshotRevision: append ? previous.snapshotRevision : '0',
        beforeRevision: append ? previous.nextBeforeRevision : '0' });
      if (version === epoch && query === timelineQuery && !disposed) emit({ timeline: mergeTimeline(previous, page, append) });
    } catch (error) {
      if (version === epoch && query === timelineQuery && !disposed) emit({ timeline: { ...previous, busy: false, error: error.message } });
    }
  }
  function apply(snapshot) {
    if (!sameScope(snapshot, state.scope)) throw new Error('Conversation scope mismatch.');
    if (state.snapshot && BigInt(snapshot.conversationRevision || '0') < BigInt(state.snapshot.conversationRevision || '0')) return;
    emit({ snapshot, historyMore: historyId ? state.historyMore : !!snapshot.historyCounts?.hasOlderHistory });
    void providers();
    if (state.timeline.snapshotRevision === '0' && !state.timeline.busy && !state.timeline.error) void timeline();
  }
  function connect() {
    stopWatch?.();
    if (!state.scope.conversationId || state.connection === 'Unavailable') return;
    const version = epoch;
    stopWatch = watchConversation({ scope: { ...state.scope }, request, EventSource, schedule, cancel,
      apply: snapshot => { if (version === epoch) apply(snapshot); },
      activity: activity => {
        if (version === epoch && !state.activities.some(item => item.cursor === activity.cursor)) emit({ activities: [...state.activities, activity].slice(-100) });
      },
      status: connection => { if (version === epoch) emit({ connection }); },
      error: error => { if (version === epoch) showError(error); },
    });
  }
  async function select(scope) {
    const previous = state.scope;
    const next = cleanScope(scope);
    reset(next);
    emit({ projects: previous.workspaceId === next.workspaceId ? state.projects : [], conversations: previous.projectId === next.projectId ? state.conversations : [], pages: {} });
    const version = epoch;
    await refresh();
    if (version === epoch && state.connection !== 'Unavailable') connect();
  }
  async function accepted(command, result) {
    const receipt = result.receipt || result;
    emit({ lastReceipt: receipt });
    if (result.workspace) await select({ workspaceId: result.workspace.id });
    else if (result.project) await select({ workspaceId: result.project.workspaceId, projectId: result.project.id });
    else if (result.conversation) await select({ workspaceId: result.conversation.workspaceId, projectId: result.conversation.projectId, conversationId: result.conversation.id });
    else if (sameScope(command.body, state.scope)) {
      const version = epoch;
      try {
        const result = await request('GetConversationSnapshot', { ...state.scope });
        if (version === epoch) apply(result.snapshot);
      } catch (error) { if (version === epoch) showError(error); }
    }
  }
  async function retry() {
    if (!state.pending || state.sending) return false;
    const command = structuredClone(state.pending);
    emit({ sending: true, error: '' });
    try {
      const result = await request(command.method, command.body);
      persist(pendingKey, null);
      emit({ pending: null });
      await accepted(command, result);
      return true;
    } catch (error) {
      if (isDefinitive(error)) { persist(pendingKey, null); emit({ pending: null }); }
      showError(error);
      return false;
    } finally { emit({ sending: false }); }
  }
  async function submit(method, body) {
    if (state.pending || state.sending || state.busy || !knownCommands.has(method)) return false;
    const pending = { method, body: { ...structuredClone(body), idempotencyKey: crypto.randomUUID() } };
    emit({ pending });
    persist(pendingKey, pending);
    return retry();
  }
  async function older() {
    if (!state.snapshot || state.historyBusy) return;
    const version = epoch;
    historyId ||= state.snapshot.historySnapshotId;
    emit({ historyBusy: true });
    try {
      const result = await request('ListConversationHistory', { ...state.scope, snapshotId: historyId, beforeSequence: before, limit: 50 });
      if (version !== epoch) return;
      before = result.nextBeforeSequence || '0';
      const records = [...(result.records || []), ...state.history];
      emit({ history: [...new Map(records.map(record => [record.id, record])).values()], historyMore: !!result.hasMore });
    } catch (error) {
      if (version === epoch) {
        if (error.reason === 'ERROR_REASON_HISTORY_SNAPSHOT_EXPIRED') { historyId = ''; before = '0'; emit({ history: [] }); }
        showError(error);
      }
    } finally { if (version === epoch) emit({ historyBusy: false }); }
  }
  return {
    get state() { return state; },
    subscribe(listener) { listeners.add(listener); listener(state); return () => listeners.delete(listener); },
    async start() { await select(state.scope); }, select, refresh,
    async more(kind) { try { await loadPage(kind, true); } catch (error) { showError(error); } },
    submit, retry, older, timeline,
    pause() { stopWatch?.(); stopWatch = undefined; emit({ connection: 'Paused · work continues' }); },
    reconnect: connect,
    dismissError() { emit({ error: '' }); },
    dispose() { stopWatch?.(); disposed = true; epoch++; catalog++; listeners.clear(); },
  };
}
