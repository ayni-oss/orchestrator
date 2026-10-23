import test from 'node:test';
import assert from 'node:assert/strict';
import { taskActions, taskGuidance, publicationCommand, publicationLink } from './tasks.mjs';
import { makeCommand } from './state.mjs';
const scope = { workspaceId: 'w', projectId: 'p', conversationId: 'c' };
const passed = { attemptId: 'a', state: 'completed', executionOutcome: 'completed', quality: { state: 'passed' }, patch: 'accepted', patchDigest: 'patch', qualityDigest: 'quality' };
const preview = { request: { ...scope, taskId: 't', attemptId: 'a', expectedPatchDigest: 'patch', expectedQualityDigest: 'quality' } };

test('publication authorization retains exact scope and evidence across response-loss retries', () => {
  const command = publicationCommand(scope, 't', passed, preview);
  const pending = makeCommand(command.method, command.body, 'original-key');
  const restored = JSON.parse(JSON.stringify(pending));
  assert.deepEqual(restored.body, { ...preview.request, idempotencyKey: 'original-key' });
  assert.throws(() => publicationCommand({ ...scope, conversationId: 'foreign' }, 't', passed, preview));
  assert.throws(() => publicationCommand(scope, 'foreign', passed, preview));
  for (const edited of [{ attemptId: 'replacement' }, { qualityDigest: 'changed' }, { patchDigest: 'changed' }, { publication: { state: 'queued' } }, { quality: { state: 'failed' } }]) {
    assert.throws(() => publicationCommand(scope, 't', { ...passed, ...edited }, preview));
  }
});
test('controls fence terminal work and never offer another publication after uncertainty or cancellation', () => {
  for (const state of ['queued', 'claimed', 'reporting']) assert.equal(taskActions({ state }).cancelCoding, true);
  for (const state of ['completed', 'failed', 'interrupted', 'cancelling', 'cancelled']) assert.equal(taskActions({ state }).cancelCoding, false);
  for (const state of ['queued', 'claimed', 'prepared']) assert.equal(taskActions({ publication: { state } }).cancelPublication, true);
  for (const state of ['creating', 'uncertain', 'draft', 'blocked', 'cancelled']) {
    const attempt = { ...passed, publication: { state } };
    assert.equal(taskActions(attempt).cancelPublication, false);
    assert.equal(taskActions(attempt).reviewPublication, false);
  }
  assert.equal(taskActions(passed).reviewPublication, true);
  assert.match(taskGuidance({ attemptId: 'a' }, { state: 'interrupted', providerState: 'uncertain' }), /may have incurred a charge/);
  assert.match(taskGuidance({ attemptId: 'a' }, { ...passed, quality: { state: 'failed' } }), /Coding results remain retained/);
  assert.match(taskGuidance({ attemptId: 'a' }, { ...passed, publication: { state: 'creating' } }), /do not create another/);
});
test('external PR links must match the recorded repository and number exactly', () => {
  const p = { repository: 'https://github.com/example/repo.git', prNumber: '9007199254740993', prUrl: 'https://github.com/example/repo/pull/9007199254740993' };
  assert.equal(publicationLink(p), p.prUrl);
  for (const prUrl of ['javascript:alert(1)', 'https://other.example/1', 'https://github.com/another/repo/pull/9007199254740993']) assert.equal(publicationLink({ ...p, prUrl }), '');
  assert.equal(publicationLink({ ...p, prNumber: '0' }), '');
  assert.equal(publicationLink({ ...p, repository: 'https://github.com/../repo.git', prUrl: 'https://github.com/../repo/pull/9007199254740993' }), '');
});
