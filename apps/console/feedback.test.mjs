import test from 'node:test';
import assert from 'node:assert/strict';
import { createFeedbackList } from './feedback.mjs';

const original = { workspaceId: 'workspace', projectId: 'project', conversationId: 'conversation' };
const item = (id, scope = original) => ({ ...scope, id, taskId: 'task', attemptId: 'attempt', state: 'open', summary: 'Inspect retained work.' });
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('feedback pagination stays bounded and distinguishes project from conversation scope', async () => {
  const calls = [];
  const api = async (method, body) => {
    calls.push({ method, body });
    return body.afterId ? { feedback: [item('a'), item('b')] } : { feedback: [item('a')], nextAfterId: 'a' };
  };
  const conversation = createFeedbackList({ getScope: () => original, api });
  await conversation.load(); await conversation.load(true);
  assert.deepEqual(conversation.state().items.map(value => value.id), ['a', 'b']);
  assert.deepEqual(calls[0], { method: 'ListTaskFeedback', body: { ...original, afterId: '', limit: 20 } });
  assert.equal(calls[1].body.afterId, 'a');
  const project = createFeedbackList({ getScope: () => original, api, projectWide: true });
  await project.load();
  assert.equal(Object.hasOwn(calls[2].body, 'conversationId'), false);
});

test('selection changes fence old reads and errors, including return to the same scope', async () => {
  let scope = original;
  const requests = [];
  const errors = [];
  const list = createFeedbackList({ getScope: () => scope, api: () => { const request = deferred(); requests.push(request); return request.promise; }, failed: error => errors.push(error) });
  const old = list.load();
  scope = { ...original, conversationId: 'other' }; list.reset();
  const current = list.load(); requests[1].resolve({ feedback: [item('current', scope)] }); await current;
  requests[0].resolve({ feedback: [item('old')] }); await old;
  assert.deepEqual(list.state().items.map(value => value.id), ['current']);
  const stale = list.load(); list.reset(); scope = original; list.reset();
  requests[2].reject(new Error('old scope failed')); await stale;
  assert.equal(errors.length, 0); assert.deepEqual(list.state().items, []);
});

test('new refresh wins and invalid scope or read failure clears stale feedback', async () => {
  const requests = [];
  const errors = [];
  const list = createFeedbackList({ getScope: () => original, api: () => { const request = deferred(); requests.push(request); return request.promise; }, failed: error => errors.push(error) });
  const first = list.load(), second = list.load();
  requests[1].resolve({ feedback: [item('fresh')] }); await second;
  requests[0].resolve({ feedback: [item('stale')] }); await first;
  assert.equal(list.state().items[0].id, 'fresh');
  const invalid = list.load(); requests[2].resolve({ feedback: [item('foreign', { ...original, projectId: 'foreign' })] }); await invalid;
  assert.equal(list.state().error, true); assert.deepEqual(list.state().items, []);
  assert.match(errors[0].message, /selected scope/);
});
