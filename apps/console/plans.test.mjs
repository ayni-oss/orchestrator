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
