import test from 'node:test';
import assert from 'node:assert/strict';
import { createWorkspace, cleanScope, isDefinitive, turnStatus } from '../src/workspace.mjs';
import { watchConversation } from '../src/watch.mjs';
import { consoleLink, count, elapsed, request as gateway } from '../src/api.mjs';

const scope = { workspaceId: 'workspace-1', projectId: 'project-1', conversationId: 'conversation-1' };
const snapshot = (revision = '1', selected = scope) => ({ ...selected, conversationRevision: revision, resumeCursor: `cursor-${revision}`, title: 'Conversation', recentMessages: [], historyCounts: { messages: '0', turns: '0' } });
const tick = async () => { for (let i = 0; i < 8; i++) await new Promise(resolve => setImmediate(resolve)); };
class Events {
  static sources = [];
  constructor(url) { this.url = url; this.handlers = {}; Events.sources.push(this); }
  addEventListener(name, handler) { this.handlers[name] = handler; }
  close() { this.closed = true; }
  frame(value) { this.handlers.frame({ data: JSON.stringify(value) }); }
  fault(value) { this.handlers.fault({ data: JSON.stringify(value) }); }
}
const memory = () => {
  const values = new Map();
  return { getItem: key => values.get(key) || null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
};
function fixture(overrides = {}) {
  const calls = [];
  const request = async (method, body) => {
    calls.push({ method, body: structuredClone(body) });
    if (overrides[method]) return overrides[method](body);
    if (method === 'ListWorkspaces') return { workspaces: [{ id: scope.workspaceId, displayName: 'Workspace' }] };
    if (method === 'ListProjects') return { projects: [{ id: scope.projectId, displayName: 'Project', workspaceId: scope.workspaceId }] };
    if (method === 'ListConversations') return { conversations: [{ id: scope.conversationId, title: 'Conversation' }] };
    if (method === 'GetConversationSnapshot') return { snapshot: snapshot() };
    if (method === 'ListProviderCalls') return { calls: [] };
    return { receipt: { commandId: body.idempotencyKey } };
  };
  return { request, calls };
}
const clientFor = (request, storage = memory()) => createWorkspace({ request, storage, EventSource: Events, initialScope: scope });

test('ambiguous admission survives reload and retries identical frozen bytes', async () => {
  const storage = memory(); let fail = true;
  const { request, calls } = fixture({ SendMessage: () => { if (fail) throw new Error('Response lost'); return { receipt: { commandId: 'accepted-once' } }; } });
  const first = clientFor(request, storage);
  await first.start();
  const body = { ...scope, content: 'Original message' };
  assert.equal(await first.submit('SendMessage', body), false);
  body.content = 'Later edit';
  const original = structuredClone(first.state.pending);
  first.dispose();
  const reloaded = clientFor(request, storage);
  assert.deepEqual(reloaded.state.pending, original);
  await reloaded.start(); fail = false;
  assert.equal(await reloaded.retry(), true);
  const admissions = calls.filter(call => call.method === 'SendMessage');
  assert.equal(admissions.length, 2);
  assert.deepEqual(admissions[0], admissions[1]);
  assert.equal(reloaded.state.pending, null);
  assert.equal(reloaded.state.lastReceipt.commandId, 'accepted-once');
  reloaded.dispose();
});

test('definitive non-admission releases the key; ambiguous capacity preserves it', async () => {
  let bounded = true;
  const { request } = fixture({ SendMessage: () => { throw Object.assign(new Error('Capacity'), { code: 'ResourceExhausted', reason: bounded ? 'ERROR_REASON_BOUND_EXCEEDED' : '' }); } });
  const client = clientFor(request); await client.start();
  await client.submit('SendMessage', { ...scope, content: 'Test' });
  assert.equal(client.state.pending, null);
  bounded = false;
  await client.submit('SendMessage', { ...scope, content: 'Test' });
  assert.equal(client.state.pending.method, 'SendMessage');
  assert.equal(await client.submit('SendMessage', { ...scope, content: 'Duplicate' }), false);
  client.dispose();
});

test('selecting another conversation fences stale watch and history results', async () => {
  let completeHistory;
  const { request } = fixture({
    ListConversations: () => ({ conversations: [{ id: 'conversation-1' }, { id: 'conversation-2' }] }),
    ListConversationHistory: () => new Promise(resolve => { completeHistory = resolve; }),
  });
  const client = clientFor(request); await client.start();
  const old = Events.sources.at(-1); old.frame({ snapshot: { ...snapshot(), historySnapshotId: 'history-1', historyCounts: { hasOlderHistory: true } } }); await tick();
  const pending = client.older();
  const next = { ...scope, conversationId: 'conversation-2' };
  await client.select(next);
  old.frame({ snapshot: { ...snapshot('99'), title: 'Wrong conversation' } });
  completeHistory({ records: [{ id: 'private-message', contentOrOutcome: 'Must not leak' }] });
  await pending; await tick();
  assert.equal(old.closed, true);
  assert.equal(client.state.snapshot, null);
  assert.deepEqual(client.state.history, []);
  Events.sources.at(-1).frame({ snapshot: snapshot('1', next) }); await tick();
  assert.equal(client.state.snapshot.conversationId, next.conversationId);
  client.dispose();
});

test('catalog lookup follows pagination to restore a saved selection', async () => {
  const { request, calls } = fixture({ ListConversations: body => body.afterId ? { conversations: [{ id: scope.conversationId }] } : { conversations: [{ id: 'earlier-conversation' }], nextAfterId: 'page-2' } });
  const client = clientFor(request); await client.start();
  assert.equal(client.state.connection, 'Connecting');
  assert.equal(client.state.conversations.length, 2);
  assert.equal(calls.filter(call => call.method === 'ListConversations')[1].body.afterId, 'page-2');
  client.dispose();
});

test('late catalog completion cannot overwrite a newer project selection', async () => {
  let complete;
  const { request } = fixture({ ListConversations: body => body.projectId === 'project-1' ? new Promise(resolve => { complete = resolve; }) : { conversations: [] }, ListProjects: () => ({ projects: [{ id: 'project-1' }, { id: 'project-2' }] }) });
  const client = clientFor(request);
  const first = client.start(); await tick();
  await client.select({ workspaceId: scope.workspaceId, projectId: 'project-2' });
  complete({ conversations: [{ id: 'stale' }], nextAfterId: 'stale-page' }); await first;
  assert.equal(client.state.scope.projectId, 'project-2');
  assert.deepEqual(client.state.conversations, []);
  assert.equal(client.state.pages.conversations, '');
  client.dispose();
});

test('snapshot revisions stay exact above JavaScript integer precision', async () => {
  const { request } = fixture(); const client = clientFor(request); await client.start();
  const source = Events.sources.at(-1);
  source.frame({ snapshot: snapshot('9007199254740993') });
  source.frame({ snapshot: snapshot('9007199254740992') }); await tick();
  assert.equal(client.state.snapshot.conversationRevision, '9007199254740993');
  client.dispose();
});

test('watch replays from last applied cursor after a failed projection refresh', async () => {
  let restart; const errors = [], applied = [];
  const stop = watchConversation({ scope, EventSource: Events, apply: value => applied.push(value), activity() {}, status() {}, error: error => errors.push(error), request: async () => { throw new Error('Projection unavailable'); }, schedule: callback => { restart = callback; }, cancel() {} });
  const first = Events.sources.at(-1);
  first.frame({ snapshot: snapshot('1') }); await tick();
  first.frame({ update: { cursor: 'must-not-advance' } }); await tick();
  assert.equal(errors.length, 1); assert.equal(applied.length, 1);
  restart(); assert.match(Events.sources.at(-1).url, /afterCursor=cursor-1/);
  stop();
});

test('watch cursor expiry restarts atomically and foreign scope never applies', async () => {
  let restart; const errors = [], applied = [];
  const stop = watchConversation({ scope, EventSource: Events, apply: value => applied.push(value), activity() {}, status() {}, error: error => errors.push(error), request() {}, schedule: callback => { restart = callback; }, cancel() {} });
  const first = Events.sources.at(-1); first.frame({ snapshot: snapshot('1') }); await tick();
  first.fault({ reason: 'ERROR_REASON_WATCH_CURSOR_EXPIRED' }); restart();
  const next = Events.sources.at(-1); assert.doesNotMatch(next.url, /afterCursor/);
  next.frame({ snapshot: snapshot('2', { ...scope, projectId: 'foreign' }) }); await tick();
  assert.equal(applied.length, 1); assert.equal(errors.length, 1);
  stop();
});

test('browser storage failures keep an explicit in-memory retry warning', async () => {
  const { request } = fixture({ SendMessage: () => { throw new Error('Network lost'); } });
  const storage = { getItem() { throw new Error(); }, setItem() { throw new Error(); }, removeItem() { throw new Error(); } };
  const client = clientFor(request, storage); await client.start();
  await client.submit('SendMessage', { ...scope, content: 'Retain me' });
  assert.match(client.state.storageWarning, /Keep this tab open/);
  assert.equal(client.state.pending.body.content, 'Retain me');
  client.dispose();
});

test('scope, status and metrics preserve unknown values and bounded selection', () => {
  assert.deepEqual(cleanScope({ projectId: 'orphan', conversationId: 'orphan' }), {});
  assert.deepEqual(cleanScope({ ...scope, workspaceId: '<script>' }), {});
  assert.match(consoleLink(scope), /conversationId=conversation-1/);
  assert.equal(count(undefined), 'Unknown'); assert.equal(count('0'), '0');
  assert.equal(count('9007199254740993').replaceAll(/\D/g, ''), '9007199254740993');
  assert.equal(elapsed('2026-10-25T10:00:00Z', Date.parse('2026-10-25T10:01:05Z')), '1m 5s');
  assert.equal(isDefinitive({ code: 'Unavailable' }), false);
  assert.equal(turnStatus({ nonterminalTurns: [{}], currentActivity: { waitingOn: 'WAITING_ON_PROVISIONING' } }), 'Preparing an isolated session');
});


test('oversized UTF-8 commands fail before admission without becoming ambiguous retries', async t => {
  const fetch = t.mock.method(globalThis, 'fetch', async () => { throw new Error('Must not send'); });
  await assert.rejects(gateway('SendMessage', { ...scope, content: '😀'.repeat(16384) }), error => isDefinitive(error));
  assert.equal(fetch.mock.callCount(), 0);
});

test('gateway body-limit rejection is definitive; unreadable responses remain ambiguous', async t => {
  const fetch = t.mock.method(globalThis, 'fetch', async () => new Response('Command too large', { status: 413 }));
  await assert.rejects(gateway('SendMessage', scope), error => isDefinitive(error));
  fetch.mock.mockImplementation(async () => new Response('Upstream disconnected', { status: 502 }));
  await assert.rejects(gateway('SendMessage', scope), error => !isDefinitive(error) && /original command/.test(error.message));
});

test('timeline pages keep a frozen boundary and deduplicate replayed entries', async () => {
  const entry = revision => ({ activity: { id: `activity-${revision}`, conversationRevision: revision, stage: 'ACTIVITY_STAGE_EXECUTION' } });
  const { request, calls } = fixture({ ListExecutionTimeline: body => body.beforeRevision === '0'
    ? { entries: [entry('9007199254740993')], snapshotRevision: '9007199254740993', nextBeforeRevision: '9007199254740993', retainedFromRevision: '1' }
    : { entries: [entry('9007199254740993'), entry('1')], snapshotRevision: body.snapshotRevision, retainedFromRevision: '1' } });
  const client = clientFor(request); await client.start();
  Events.sources.at(-1).frame({ snapshot: snapshot('9007199254740993') }); await tick();
  await client.timeline(true);
  assert.equal(client.state.timeline.entries.length, 2);
  assert.equal(client.state.timeline.nextBeforeRevision, '0');
  const pages = calls.filter(call => call.method === 'ListExecutionTimeline');
  assert.equal(pages[1].body.snapshotRevision, '9007199254740993');
  assert.equal(pages[1].body.beforeRevision, '9007199254740993');
  client.dispose();
});

test('timeline refresh fences older pages and scope changes fence late failures', async () => {
  let finishPage, failPage, count = 0;
  const { request } = fixture({
    ListConversations: () => ({ conversations: [{ id: 'conversation-1' }, { id: 'conversation-2' }] }),
    ListExecutionTimeline: () => {
      count++;
      if (count === 2) return new Promise(resolve => { finishPage = resolve; });
      if (count === 4) return new Promise((_, reject) => { failPage = reject; });
      return { entries: [{ activity: { id: `activity-${count}` } }], snapshotRevision: String(count), nextBeforeRevision: '1' };
    },
  });
  const client = clientFor(request); await client.start();
  Events.sources.at(-1).frame({ snapshot: snapshot() }); await tick();
  const older = client.timeline(true);
  await client.timeline();
  finishPage({ entries: [{ activity: { id: 'stale-page' } }], snapshotRevision: '1' }); await older;
  assert.equal(client.state.timeline.entries[0].activity.id, 'activity-3');
  const stale = client.timeline();
  await client.select({ ...scope, conversationId: 'conversation-2' });
  failPage(new Error('Private error from previous scope')); await stale;
  assert.deepEqual(client.state.timeline.entries, []);
  assert.equal(client.state.timeline.error, '');
  client.dispose();
});

test('a failed timeline read preserves accepted history and can be retried', async () => {
  let fail = false;
  const { request } = fixture({ ListExecutionTimeline: () => {
    if (fail) throw new Error('Unavailable');
    return { entries: [{ activity: { id: 'retained' } }], snapshotRevision: '1' };
  } });
  const client = clientFor(request); await client.start();
  Events.sources.at(-1).frame({ snapshot: snapshot() }); await tick();
  fail = true; await client.timeline();
  assert.equal(client.state.timeline.entries[0].activity.id, 'retained');
  assert.equal(client.state.timeline.error, 'Unavailable');
  fail = false; await client.timeline();
  assert.equal(client.state.timeline.error, '');
  client.dispose();
});
