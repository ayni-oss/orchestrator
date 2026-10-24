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
