import test from 'node:test';
import assert from 'node:assert/strict';
import { newerOrEqual, makeCommand, appendActivity, scopeKey } from './state.mjs';

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
