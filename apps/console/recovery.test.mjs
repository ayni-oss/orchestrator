import test from 'node:test';
import assert from 'node:assert/strict';
import { createRecoveryHistory } from './recovery.mjs';
const scope = { workspaceId:'w',projectId:'p',conversationId:'c',taskId:'t',attemptId:'a' };
test('recovery history paginates, deduplicates and grants only scoped reads', async () => {
  const calls=[];let result;
  const history=createRecoveryHistory({ api:async(method,body)=>{calls.push({method,body});return {records:[{...scope,id:'r'}],nextAfterId:calls.length===1?'r':''};},changed:s=>{result=s;} });
  history.select(scope);await history.load();await history.load(true);
  assert.equal(result.records.length,1);assert.equal(result.next,'');
  assert.deepEqual(calls.map(c=>c.method),['ListTaskRecovery','ListTaskRecovery']);assert.equal(calls[1].body.afterId,'r');
});
test('late attempt responses and failures cannot render after navigation', async () => {
  let finish;const changes=[];
  const history=createRecoveryHistory({ api:()=>new Promise(resolve=>{finish=resolve;}),changed:s=>changes.push(s) });
  history.select(scope);const read=history.load();history.select({...scope,attemptId:'other'});finish({records:[{...scope,id:'old'}]});await read;
  assert.equal(changes.length,0);
});
test('foreign recovery records fail closed and clear displayed history', async () => {
  let result;let error;
  const history=createRecoveryHistory({api:async()=>({records:[{...scope,taskId:'foreign'}]}),changed:s=>{result=s;},failed:e=>{error=e;} });
  history.select(scope);await history.load();assert.equal(result.error,true);assert.deepEqual(result.records,[]);assert.ok(error);
});

test('confirmation requires exact originating attempt, explicit consent and possible-charge acknowledgement', async () => {
  const { confirmationCommand } = await import('./recovery.mjs');
  const record = { ...scope, id: 'proposal', contentDigest: 'sha256:exact', proposal: { evidence: { providerUncertain: true } } };
  assert.throws(() => confirmationCommand(scope, record, false, true));
  assert.throws(() => confirmationCommand(scope, record, true, false));
  assert.throws(() => confirmationCommand({ ...scope, attemptId: 'foreign' }, record, true, true));
  const command = confirmationCommand(scope, record, true, true);
  assert.equal(command.method, 'ConfirmTaskRecovery');
  assert.deepEqual(command.body, { workspaceId: 'w', projectId: 'p', conversationId: 'c', taskId: 't', proposalId: 'proposal', proposalDigest: 'sha256:exact', acknowledgeProviderUncertainty: true });
});

test('question-only pages include canonical answers and unchanged refresh preserves consent and drafts', async () => {
  const question = { ...scope, id: 'q', question: { text: 'Keep changes?' } };
  const answer = { ...scope, id: 'answer', answer: { questionId: 'q', text: 'Keep them.' } };
  const changes = [];
  const history = createRecoveryHistory({ api: async () => ({ records: [question], questionAnswers: [answer], nextAfterId: 'q' }), changed: s => changes.push(s) });
  history.select(scope); await history.load(); await history.load();
  assert.equal(changes.length, 1);
  assert.deepEqual(changes[0].answers, [answer]);
});

test('parent confirmation survives pages without the confirmed proposal', async () => {
  let result;
  const receipt = { proposalId: 'earlier-proposal', parentAttemptId: 'a', attemptId: 'child' };
  const history = createRecoveryHistory({ api: async () => ({ records: [{ ...scope, id: 'other-proposal' }], confirmations: [receipt] }), changed: s => { result = s; } });
  history.select(scope); await history.load();
  assert.deepEqual(result.confirmations, [receipt]);
});

test('foreign answer or confirmation clears stale authorization', async () => {
  for (const page of [
    { questionAnswers: [{ ...scope, attemptId: 'foreign', answer: { questionId: 'q' } }] },
    { confirmations: [{ parentAttemptId: 'foreign' }] },
  ]) {
    let result;
    const history = createRecoveryHistory({ api: async () => page, changed: s => { result = s; } });
    history.select(scope); await history.load(); assert.equal(result.error, true);
  }
});

test('background refresh retains the loaded prefix instead of collapsing paginated review', async () => {
  const changes = []; const calls = [];
  const history = createRecoveryHistory({ api: async (_, body) => {
    calls.push(body.afterId);
    return body.afterId ? { records: [{ ...scope, id: 'second' }] } : { records: [{ ...scope, id: 'first' }], nextAfterId: 'first' };
  }, changed: s => changes.push(s) });
  history.select(scope); await history.load(); await history.load(true); await history.load();
  assert.equal(changes.length, 2);
  assert.equal(changes[1].records.length, 2);
  assert.deepEqual(calls, ['', 'first', '', 'first']);
});
