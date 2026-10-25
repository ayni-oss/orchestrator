import test from 'node:test';
import assert from 'node:assert/strict';
import { progressForSnapshot, taskGuidance, publicationLink, turnRows, phaseElapsed } from '../src/execution.mjs';
const active = { nonterminalTurns: [{ id: 'turn-one' }] };
const progress = (sequence, generation = '1', extra = {}) => ({ turnId: 'turn-one', sessionId: 'session-one', generation, sequence, stage: 'model', round: 1, provisionalText: 'Partial', ...extra });

test('progress snapshots replace whole round buffers without replay duplication or stale generations', () => {
  const first = progressForSnapshot([], [progress('9007199254740993')], active);
  const older = progressForSnapshot(first, [progress('9007199254740992', '1', { provisionalText: 'Stale' })], active);
  assert.deepEqual(older, first);
  const duplicate = progressForSnapshot(first, [progress('9007199254740993', '1', { provisionalText: 'Changed same sequence' })], active);
  assert.deepEqual(duplicate, first);
  const nextRound = progressForSnapshot(first, [progress('9007199254740994', '1', { callId: 'next-call', round: 2, provisionalText: 'New round' })], active);
  assert.equal(nextRound[0].provisionalText, 'New round');
  const nextSession = progressForSnapshot(nextRound, [progress('1', '2', { sessionId: 'session-two', provisionalText: 'New generation' })], active);
  assert.equal(nextSession[0].provisionalText, 'New generation');
  assert.deepEqual(progressForSnapshot(nextSession, [progress('9999999999999999')], active), nextSession);
  assert.deepEqual(progressForSnapshot(first, [progress('9007199254740994', '1', { sessionId: 'foreign-session' })], active), first);
});

test('terminal acceptance removes provisional output and bounds prevent oversized untrusted progress', () => {
  assert.deepEqual(progressForSnapshot([progress('1')], [progress('2')], { nonterminalTurns: [] }), []);
  assert.deepEqual(progressForSnapshot([], [progress('1', '1', { turnId: 'another-turn' })], active), []);
  assert.deepEqual(progressForSnapshot([], [progress('invalid')], active), []);
  assert.throws(() => progressForSnapshot([], [progress('1', '1', { provisionalText: '🙂'.repeat(4097) })], active), /text bound/);
  assert.throws(() => progressForSnapshot([], Array.from({ length: 101 }, () => progress('1')), active), /record bound/);
});

test('task guidance preserves independent coding, quality, publication and recovery outcomes', () => {
  const prepared = { attemptId: 'attempt' };
  assert.match(taskGuidance(prepared, { state: 'completed', executionOutcome: 'completed', quality: { state: 'failed' }, patch: 'diff' }), /Quality did not pass/);
  assert.match(taskGuidance(prepared, { state: 'completed', quality: { state: 'passed' }, patch: 'diff' }), /authorize a draft/);
  assert.match(taskGuidance(prepared, { state: 'completed', publication: { state: 'uncertain' } }), /original request/);
  assert.match(taskGuidance(prepared, { state: 'interrupted' }), /exact recovery/);
  assert.match(taskGuidance(prepared, { state: 'queued' }), /available coding slot/);
});

test('publication links bind repository, PR identity and canonical HTTPS URL', () => {
  const publication = { repository: 'https://github.com/ayni-oss/orchestrator', prNumber: '12', prUrl: 'https://github.com/ayni-oss/orchestrator/pull/12' };
  assert.equal(publicationLink(publication), publication.prUrl);
  for (const patch of [{ prUrl: 'javascript:alert(1)' }, { repository: 'https://github.com.evil.test/ayni-oss/orchestrator' }, { prNumber: '012' }, { prUrl: 'https://github.com/other/repo/pull/12' }]) assert.equal(publicationLink({ ...publication, ...patch }), '');
});

test('turn inspector orders retained outcomes exactly above numeric precision', () => {
  assert.deepEqual(turnRows({ nonterminalTurns: [{ id: 'new', sequence: '9007199254740993' }], recentTerminalTurns: [{ id: 'old', sequence: '9007199254740992' }] }).map(turn => [turn.id, turn.active]), [['new', true], ['old', false]]);
});

test('finished stages without retained end times never display an increasing duration', () => {
  const start = '2026-01-01T00:00:00Z';
  const end = '2026-01-01T00:00:03Z';
  assert.equal(phaseElapsed(start, undefined, false, Date.parse(end)), 'Not recorded');
  assert.equal(phaseElapsed(start, undefined, true, Date.parse(end)), '3s');
  assert.equal(phaseElapsed(start, end, false, Date.parse(end) + 10000), '3s');
});
