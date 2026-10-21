import test from 'node:test';
import assert from 'node:assert/strict';
import { newerOrEqual, makeCommand, appendActivity, scopeKey, tokenCount, providerCallView } from './state.mjs';

test('revisions remain exact above the JavaScript integer limit', () => {
  assert.equal(newerOrEqual('9007199254740993', '9007199254740992'), true);
  assert.equal(newerOrEqual('9007199254740992', '9007199254740993'), false);
});
test('ambiguous submissions retain the same key, content, and parent scope through reload', () => {
  const original = makeCommand('SendMessage', { workspaceId: 'w', projectId: 'p', conversationId: 'c', content: 'original' }, 'fixed-key');
  const reloaded = JSON.parse(JSON.stringify(original));
  assert.deepEqual(reloaded, original);
  assert.equal(scopeKey(reloaded.body), 'w/p/c');
});
test('replayed activity is deduplicated and retained browser activity is bounded', () => {
  let values = [];
  for (let i = 0; i < 105; i++) values = appendActivity(values, { cursor: String(i) });
  assert.equal(values.length, 100);
  assert.equal(appendActivity(values, { cursor: '104' }), values);
  assert.equal(values[0].cursor, '5');
});

test('provider usage distinguishes missing values from reported zero and keeps integer precision', () => {
  assert.equal(tokenCount(undefined), 'Unknown');
  assert.equal(tokenCount(null), 'Unknown');
  assert.equal(tokenCount(''), 'Unknown');
  assert.equal(tokenCount('0'), '0');
  assert.equal(tokenCount('9007199254740993'), '9,007,199,254,740,993');
  assert.equal(tokenCount('not a counter'), 'Unknown');
});
test('uncertain calls retain factual binding, reservation and an explicit recovery notice', () => {
  const view = providerCallView({ provider: 'openai', model: 'frozen-model', state: 'uncertain', reservedTotalTokens: '10240', totalTokens: undefined });
  assert.equal(view.binding, 'openai / frozen-model');
  assert.match(view.usage, /total Unknown/);
  assert.match(view.reservation, /total 10,240/);
  assert.match(view.notice, /will not be retried automatically/);
});
test('in-flight cancellation never promises that an external call was revoked', () => {
  const view = providerCallView({ state: 'sending', cancellationRequested: true });
  assert.match(view.notice, /external outcome is unconfirmed/);
  assert.match(view.notice, /does not confirm remote revocation or a refund/);
});
test('ambiguous cancellation preserves exact turn, parent scope and receipt key across reload', () => {
  const command = makeCommand('CancelTurn', { workspaceId: 'w', projectId: 'p', conversationId: 'c', turnId: 't' }, 'cancel-key');
  const reloaded = JSON.parse(JSON.stringify(command));
  assert.deepEqual(reloaded, command);
  assert.equal(reloaded.body.turnId, 't');
  assert.equal(reloaded.body.idempotencyKey, 'cancel-key');
  assert.equal(scopeKey(reloaded.body), 'w/p/c');
});
test('acknowledged uncertainty preserves possible-charge warning without promising a known result', () => {
  const view = providerCallView({ state: 'uncertain', uncertaintyAcknowledged: true, cancellationRequested: true });
  assert.match(view.notice, /may have incurred a charge/);
  assert.match(view.notice, /evidence remains retained/);
  assert.match(view.notice, /will not be retried/);
  assert.equal(view.state, 'uncertain');
});
test('ambiguous uncertainty acknowledgement freezes exact evidence identity and explicit consent', () => {
  const command = makeCommand('AcknowledgeProviderUncertainty', { workspaceId: 'w', projectId: 'p', conversationId: 'c', turnId: 't', callId: 'call', requestDigest: 'sha256:original', acknowledgePossibleCharge: true }, 'ack-key');
  assert.deepEqual(JSON.parse(JSON.stringify(command)), command);
  assert.equal(command.body.acknowledgePossibleCharge, true);
  assert.equal(command.body.requestDigest, 'sha256:original');
});
test('tool activity projects allowlisted names and state without arguments or output', () => {
  const view = providerCallView({ toolInvocations: [
    { name: 'read_project_status', state: 'completed', arguments: 'private args', output: 'private output' },
    { name: 'shell', state: 'completed', output: 'private shell output' },
  ] });
  assert.deepEqual(view.tools, ['read_project_status · completed']);
  assert.equal(JSON.stringify(view).includes('private'), false);
  assert.equal(JSON.stringify(view).includes('shell'), false);
});
