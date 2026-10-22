import test from 'node:test';
import assert from 'node:assert/strict';
import { planCommand, planDraftKey } from './plans.mjs';

test('local revisions preserve the reviewed head and origin across later streamed heads', () => {
  const scope = { workspaceId: 'w', projectId: 'p', conversationId: 'c' };
  const draft = { planId: 'plan', expectedVersionId: 'reviewed', content: { title: 'Local changes' } };
  const command = planCommand(scope, draft);
  assert.equal(command.method, 'RevisePlan');
  assert.equal(command.body.expectedVersionId, 'reviewed');
  assert.equal(command.body.conversationId, 'c');
  assert.notEqual(planDraftKey(scope), planDraftKey({ ...scope, conversationId: 'other' }));
  assert.notEqual(planDraftKey(scope), planDraftKey({ ...scope, projectId: 'other' }));
  assert.equal(planCommand(scope, { content: draft.content }).method, 'CreatePlan');
});

test('approval freezes the reviewed version, digest and ownership in its command', async () => {
  const { approvalCommand } = await import('./plans.mjs');
  const { makeCommand } = await import('./state.mjs');
  const scope = { workspaceId: 'w', projectId: 'p', conversationId: 'c' };
  const version = { ...scope, planId: 'plan', id: 'reviewed', contentDigest: 'sha256:reviewed' };
  const approval = approvalCommand(scope, version);
  const pending = makeCommand(approval.method, approval.body, 'original-key');
  version.id = 'later'; version.contentDigest = 'sha256:later';
  const reloaded = JSON.parse(JSON.stringify(pending));
  assert.deepEqual(reloaded.body, { ...scope, planId: 'plan', expectedVersionId: 'reviewed', contentDigest: 'sha256:reviewed', idempotencyKey: 'original-key' });
  assert.throws(() => approvalCommand({ ...scope, conversationId: 'other' }, version), /another conversation/);
});

test('approval eligibility distinguishes scope questions, stale reviews and execution prerequisites', async () => {
  const { approvalBlocker } = await import('./plans.mjs');
  const version = { id: 'v1', content: { tasks: [{ acceptanceCriteria: ['Observable result'], unresolvedInputs: ['Verify source'], repository: '', executionLimits: '' }] } };
  const head = { currentVersionId: 'v1' };
  assert.equal(approvalBlocker(version, head), '');
  assert.match(approvalBlocker(version, { currentVersionId: 'v2' }), /newer version/);
  assert.match(approvalBlocker(version, { ...head, approvalId: 'receipt' }), /immutable/);
  assert.match(approvalBlocker(version, head, { id: 'receipt' }), /immutable/);
  assert.match(approvalBlocker({ ...version, content: { ...version.content, unresolvedQuestions: ['Which scope?'] } }, head), /Resolve scope/);
  assert.match(approvalBlocker({ ...version, content: { tasks: [] } }, head), /acceptance criteria/);
  assert.match(approvalBlocker({ ...version, content: { tasks: [{ acceptanceCriteria: [] }] } }, head), /acceptance criteria/);
});
