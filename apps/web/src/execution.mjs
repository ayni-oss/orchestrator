import { elapsed } from './api.mjs';

// Presentation uses retained identities; none of these hints grants authority.
export const emptyTasks = () => ({ items: [], nextAfterId: '', busy: false, error: '', selectedId: '', selectedAttemptId: '', detail: null, detailBusy: false, detailError: '', recovery: [], recoveryNextAfterId: '', recoveryBusy: false, recoveryError: '', confirmations: [], questionAnswers: [] });

export function taskGuidance(preparation, attempt) {
  if (!preparation) return 'Verify the approved source, choose a pinned runtime and authorize a budget in the contributor console and trusted local helper.';
  if (!preparation.attemptId) return 'Resolve the recorded preparation blockers before preparing again.';
  if (!attempt) return 'Source preparation is recorded. Use the trusted local helper to submit the frozen source and start execution.';
  if (attempt.publication?.state === 'uncertain' || attempt.publication?.state === 'creating') return 'Draft creation may have reached GitHub. The publisher reconciles the original request. Keep this publication identity.';
  if (attempt.providerState === 'uncertain') return 'The provider outcome is uncertain and may have incurred a charge. Inspect saved evidence and review recovery in the contributor console.';
  if (attempt.state === 'cancelling') return 'Cancellation is recorded. Waiting for the supervisor to confirm the worker has stopped.';
  if (attempt.publication?.state === 'draft') return 'Review the retained draft. The external pull request state is the last recorded observation.';
  if (['interrupted', 'failed', 'cancelled'].includes(attempt.state)) return 'Inspect saved evidence, then review an exact recovery proposal and allowance in the contributor console.';
  if (attempt.quality && attempt.quality.state !== 'passed') return 'Quality did not pass. Inspect the retained artifacts and review recovery. Publication requires passing quality.';
  if (attempt.state === 'completed' && !attempt.patch) return 'Coding completed without accepted changes. There is nothing to publish.';
  if (attempt.state === 'completed' && attempt.quality?.state === 'passed' && !attempt.publication) return 'Review the accepted patch and passing quality, then authorize a draft in the contributor console.';
  if (attempt.publication) return 'Publication proceeds under its separate authority. Inspect its recorded reason and outcome.';
  if (attempt.state === 'queued') return 'Waiting for an available coding slot. You can cancel this attempt.';
  if (attempt.state === 'reporting' && attempt.executionOutcome === 'completed' && !attempt.quality) return 'Coding results are accepted. Managed quality is preparing or running; waiting for quality evidence.';
  if (attempt.state === 'reporting') return 'The worker has reported. Waiting for terminal acceptance and slot release.';
  return 'The worker is executing. Live updates refresh its accepted evidence; you can cancel the attempt.';
}

export function publicationLink(publication) {
  if (!publication || !/^[1-9][0-9]*$/.test(String(publication.prNumber || ''))) return '';
  const repository = publication.repository?.replace(/\.git$/, '');
  if (!/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || '')) return '';
  const expected = `${repository}/pull/${publication.prNumber}`;
  return publication.prUrl === expected && new URL(expected).href === expected ? expected : '';
}

// Current activity may describe a task; retain independent turn rows as well.
export function turnRows(snapshot) {
  return [...(snapshot?.nonterminalTurns || []).map(turn => ({ ...turn, active: true })),
    ...(snapshot?.recentTerminalTurns || []).map(turn => ({ ...turn, active: false }))]
    .sort((a, b) => BigInt(a.sequence || '0') > BigInt(b.sequence || '0') ? -1 : 1);
}

export function progressForSnapshot(previous, incoming, snapshot) {
  if ((incoming || []).length > 100) throw new Error('Execution progress exceeds its record bound.');
  const active = new Set((snapshot?.nonterminalTurns || []).map(turn => turn.id));
  const current = new Map(previous.filter(item => active.has(item.turnId)).map(item => [item.turnId, item]));
  const accepted = [];
  for (const item of incoming || []) {
    if (!active.has(item.turnId) || !/^[0-9]+$/.test(item.sequence || '') || !/^[0-9]+$/.test(item.generation || '')) continue;
    if (new TextEncoder().encode(item.provisionalText || '').length > 16384) throw new Error('Execution progress exceeds its text bound.');
    const old = current.get(item.turnId);
    const generation = BigInt(item.generation), oldGeneration = BigInt(old?.generation || '0');
    if (old && (generation < oldGeneration || (generation === oldGeneration && old.sessionId !== item.sessionId))) {
      accepted.push(old); continue;
    }
    if (old && generation === oldGeneration && BigInt(item.sequence) <= BigInt(old.sequence)) {
      accepted.push(old); continue;
    }
    current.set(item.turnId, item);
    accepted.push(item);
  }
  return [...new Map(accepted.map(item => [item.turnId, item])).values()];
}

export function taskStage(attempt) {
  if (!attempt) return 'Awaiting execution';
  if (['queued', 'claimed', 'prepared', 'creating', 'uncertain'].includes(attempt.publication?.state)) return 'Draft publication';
  if (attempt.publication?.state === 'draft') return 'Draft ready for review';
  if (attempt.state === 'queued') return 'Waiting for a coding slot';
  if (attempt.state === 'cancelling') return 'Stopping the worker';
  if (attempt.state === 'claimed') return attempt.providerState === 'sending' ? 'Model execution' : 'Source and worker preparation';
  if (attempt.state === 'reporting' && attempt.executionOutcome === 'completed' && !attempt.quality) return 'Managed quality';
  if (attempt.state === 'reporting') return 'Terminal acceptance and cleanup';
  if (attempt.state === 'completed' && attempt.quality?.state === 'passed') return 'Awaiting publication review';
  return 'Review retained outcome';
}

export function phaseElapsed(start, end, active, now = Date.now()) {
  if (!start || (!end && !active)) return 'Not recorded';
  return elapsed(start, end ? Date.parse(end) : now);
}

export function progressRound(progress) {
  if (progress.stage === 'scripted') return 'Scripted execution · no model call';
  return progress.round ? `Model round ${progress.round}` : 'No model invocation yet';
}
