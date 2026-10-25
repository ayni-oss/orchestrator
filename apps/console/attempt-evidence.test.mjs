import test from 'node:test';
import assert from 'node:assert/strict';
import { retainedAttemptEvidence } from './attempt-evidence.mjs';

test('task evidence excludes only the live observation clock', () => {
  const original = { attempt: { generation: 2, timing: { observedAt: 'first' } } };
  const later = { attempt: { generation: 2, timing: { observedAt: 'later' } } };
  assert.deepEqual(retainedAttemptEvidence(original), retainedAttemptEvidence(later));
  assert.equal(original.attempt.timing.observedAt, 'first');
  for (const boundary of [
    'queueAcceptedAt', 'claimAcceptedAt', 'codingResultAcceptedAt',
    'qualityResultAcceptedAt', 'executionStoppedAt', 'publicationAuthorizedAt',
    'publicationClaimAcceptedAt', 'publicationResultAcceptedAt',
  ]) {
    const changed = structuredClone(later);
    changed.attempt.timing[boundary] = 'changed';
    assert.notDeepEqual(retainedAttemptEvidence(original), retainedAttemptEvidence(changed), boundary);
  }
  const changed = structuredClone(later);
  changed.attempt.generation += 1;
  assert.notDeepEqual(retainedAttemptEvidence(original), retainedAttemptEvidence(changed));
  delete changed.attempt.timing;
  assert.notDeepEqual(retainedAttemptEvidence(original), retainedAttemptEvidence(changed));
});
