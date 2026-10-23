import { mountRecovery } from './recovery.mjs';
import { scopeKey } from './state.mjs';

const $ = id => document.getElementById(id);
const text = value => value == null || value === '' ? 'Not recorded' : Array.isArray(value) ? value.join('\n') || 'None' : String(value);
const human = value => text(value).replaceAll('_', ' ');
const codingActive = ['queued', 'claimed', 'reporting'];
const publicationActive = ['queued', 'claimed', 'prepared'];

// Display hints never grant authority. Every command rechecks current state in PostgreSQL.
export function taskActions(attempt) {
  return {
    cancelCoding: codingActive.includes(attempt?.state),
    reviewPublication: attempt?.state === 'completed' && attempt.executionOutcome === 'completed'
      && attempt.quality?.state === 'passed' && !!attempt.patch && !!attempt.patchDigest
      && !!attempt.qualityDigest && !attempt.publication,
    cancelPublication: publicationActive.includes(attempt?.publication?.state),
  };
}
export function taskGuidance(preparation, attempt) {
  if (!preparation) return 'Verify the approved source, select a pinned runtime and authorize an execution budget with the local helper.';
  if (!preparation.attemptId) return 'Preparation is blocked. Review the recorded blockers and resolve the missing inputs before preparing again.';
  if (!attempt) return 'Prepared. Use the local helper to freeze and submit the source snapshot to start coding.';
  if (attempt.publication?.state === 'uncertain' || attempt.publication?.state === 'creating') return 'Draft creation may have reached GitHub. The publisher reconciles the original request; do not create another publication.';
  if (attempt.providerState === 'uncertain') return 'The coding provider outcome is uncertain and may have incurred a charge. Keep the retained evidence. Coding is not retried automatically.';
  if (attempt.state === 'cancelling') return 'Cancellation is recorded. Waiting for the supervisor to confirm that the owned worker has stopped.';
  if (attempt.publication?.state === 'cancelled') return 'Publication was cancelled. A branch pushed before cancellation may remain.';
  if (attempt.publication?.state === 'blocked') return 'Publication is blocked. Inspect the retained reason and remote branch state; this attempt does not authorize another draft.';
  if (attempt.publication?.state === 'draft') return 'The retained pull request is available for review. Its external state is the last recorded observation.';
  if (['interrupted', 'failed', 'cancelled'].includes(attempt.state)) return 'Inspect the retained outcome and changes. Review saved evidence and record a recovery proposal with an additional allowance. Continuation is not available yet.';
  if (attempt.state === 'completed' && !attempt.patch) return 'Coding completed without accepted changes. There is nothing to publish.';
  if (attempt.quality && attempt.quality.state !== 'passed') return 'Quality did not pass. Coding results remain retained. Publication is unavailable. Record a recovery proposal for review; it will not start repair.';
  if (taskActions(attempt).reviewPublication) return 'Review the accepted patch and passing quality evidence, then review and authorize a draft publication.';
  if (attempt.publication) return 'Publication is in progress under its separate authority. Coding and quality will not rerun.';
  if (attempt.state === 'queued') return 'Waiting for the serial coding slot. Only one coding task runs at a time.';
  if (attempt.state === 'reporting') return 'The worker has reported. Waiting for terminal confirmation and slot release.';
  return 'Coding and managed quality are in progress. Refresh to inspect retained progress.';
}
export function publicationLink(publication) {
  if (!publication || !/^[1-9][0-9]*$/.test(String(publication.prNumber || ''))) return '';
  const repository = publication.repository?.replace(/\.git$/, '');
  if (!/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || '')) return '';
  const expected = `${repository}/pull/${publication.prNumber}`;
  return publication.prUrl === expected && new URL(expected).href === expected ? expected : '';
}
export function publicationCommand(scope, taskId, attempt, preview) {
  const request = preview.request;
  if (scopeKey(scope) !== scopeKey(request) || request.taskId !== taskId
      || request.attemptId !== attempt?.attemptId || !taskActions(attempt).reviewPublication
      || request.expectedPatchDigest !== attempt.patchDigest || request.expectedQualityDigest !== attempt.qualityDigest) {
    throw new Error('Publication evidence changed. Refresh the task and review publication again.');
  }
  return { method: 'StartTaskPublication', body: { ...request } };
}

export function mountTasks({ getScope, api, submit, showError, saved, store, controls }) {
  const recovery = mountRecovery({ api, submit, showError, controls });
  let owner = '';
  let selectedId = '';
  let next = '';
  let listQuery = 0;
  let detailQuery = 0;
  let reviewing = 0;
  let attempt;
  let preview;
  let timer;
  let busy = false;
  let reloadRequested = false;
  let disposed = false;
  const selectionKey = () => `task-selection:${owner}`;
  function row(parent, label, value) {
    const dt = document.createElement('dt'); dt.textContent = label;
    const dd = document.createElement('dd'); dd.textContent = text(value);
    parent.append(dt, dd);
  }
  function group(title, entries) {
    const section = document.createElement('section'); section.className = 'task-stage';
    const h = document.createElement('h3'); h.textContent = title;
    const dl = document.createElement('dl'); for (const [label, value] of entries) row(dl, label, value);
    section.append(h, dl); $('task-detail').append(section); return section;
  }
  function disclosure(parent, title, value, key) {
    const details = document.createElement('details'); details.dataset.taskEvidence = key;
    const summary = document.createElement('summary'); summary.textContent = title;
    const pre = document.createElement('pre'); pre.textContent = text(value);
    details.append(summary, pre); parent.append(details);
  }
  function resetReview() {
    preview = undefined; reviewing++;
    $('task-publication-review').replaceChildren();
  }
  function action(parent, title, method, body) {
    const button = document.createElement('button'); button.type = 'button'; button.textContent = title;
    button.dataset.taskAction = method;
    button.onclick = () => submit(method, body);
    parent.append(button);
  }
  function render(task, preparation, run) {
    const opened = new Set([...$('task-detail').querySelectorAll('details[open]')].map(d => d.dataset.taskEvidence));
    $('task-detail').replaceChildren();
    $('task-status').textContent = taskGuidance(preparation, run);
    $('task-status').dataset.taskId = task.id;
    $('task-status').dataset.codingState = run?.state || '';
    $('task-status').dataset.qualityState = run?.quality?.state || '';
    $('task-status').dataset.publicationState = run?.publication?.state || '';
    const scope = { ...getScope(), taskId: task.id };
    group(task.specification.objective, [['Task', task.id], ['Approved scope', task.specification.scope], ['Acceptance criteria', task.specification.acceptanceCriteria]]);
    const source = preparation?.inputs;
    const setup = group('Preparation', [
      ['Status', !preparation ? 'Not prepared' : preparation.attemptId ? 'Prepared' : 'Blocked'],
      ['Preparation blockers', preparation ? preparation.blockers || [] : task.executionBlockers],
      ['Repository', source?.repository || task.specification.repository],
      ['Source reference', source?.sourceRef || task.specification.sourceRef], ['Source commit', source?.sourceCommit],
      ['Runtime image', source?.runtimeImage], ['Time budget (seconds)', source?.maxSeconds], ['Token budget', source?.maxTokens],
    ]);
    disclosure(setup, 'Origin and frozen preparation evidence', JSON.stringify({ origin: task, preparation }, null, 2), 'origin');
    const coding = group('Coding', [
      ['Status', human(run?.state || (preparation?.attemptId ? 'not_started' : 'awaiting_preparation'))],
      ['Outcome', human(run?.executionOutcome || 'pending')], ['Provider', human(run?.providerState || 'not_started')],
      ['Attempt', preparation?.attemptId], ['Reason', run?.safeReason],
    ]);
    if (run) {
      disclosure(coding, 'Accepted patch', run.patch || 'No accepted changes.', 'patch');
      disclosure(coding, 'Worker and source identities', JSON.stringify({ generation: run.generation, podUid: run.podUid, sourceCommit: run.sourceCommit, runtimeImage: run.runtimeImage, configurationRevisionId: run.configurationRevisionId, patchDigest: run.patchDigest }, null, 2), 'worker');
      if (taskActions(run).cancelCoding) {
        const notice = document.createElement('p'); notice.textContent = 'Cancellation fences the worker. A provider request already sent may have incurred a charge.'; coding.append(notice);
        action(coding, 'Cancel task execution', 'CancelTaskExecution', { ...scope, attemptId: run.attemptId });
      }
    }
    const q = run?.quality;
    const quality = group('Managed quality', [
      ['Status', human(q?.state || 'pending')], ['Reason', q?.safeReason],
      ['Completed targets', q ? `${q.completedTargets || '0'} / ${q.expectedTargets || '0'}` : 'Not recorded'],
      ['Exit code', q?.exitCode], ['Runtime platform', q?.runtimePlatform], ['Evidence digest', run?.qualityDigest],
    ]);
    if (q) {
      disclosure(quality, 'Quality identities and completion evidence', JSON.stringify({ ...q, artifact: undefined }, null, 2), 'quality');
      disclosure(quality, 'Retained quality artifacts', q.artifact, 'artifacts');
    }
    const p = run?.publication;
    const publication = group('Draft publication', [
      ['Status', human(p?.state || 'not_authorized')], ['Reason', p?.safeReason], ['Repository', p?.repository],
      ['Branch', p?.branch], ['Base branch', p?.baseBranch], ['Commit', p?.commitSha], ['Last recorded PR state', p?.prState],
    ]);
    const link = publicationLink(p);
    if (link) { const a = document.createElement('a'); a.href = link; a.textContent = 'Open retained pull request'; a.target = '_blank'; a.rel = 'noopener noreferrer'; publication.append(a); }
    if (p) disclosure(publication, 'Retained publication identity', JSON.stringify(p, null, 2), 'publication');
    if (taskActions(run).cancelPublication) {
      const notice = document.createElement('p'); notice.textContent = 'Cancel before draft creation begins. A branch already pushed may remain.'; publication.append(notice);
      action(publication, 'Cancel draft publication', 'CancelTaskPublication', { ...scope, attemptId: run.attemptId });
    }
    if (taskActions(run).reviewPublication) {
      const button = document.createElement('button'); button.type = 'button'; button.textContent = 'Review draft publication'; button.dataset.taskAction = 'ReviewTaskPublication';
      button.onclick = () => review().catch(error => { if (error.code === 'FailedPrecondition') showError(new Error('Publication review requires completed changes, passing managed quality, a public GitHub HTTPS repository and a branch source reference. Inspect the task inputs before trying again.')); else showError(error); }); publication.append(button);
    }
    const helper = group('Local source preparation', [['Next step', 'Source verification and source upload use the trusted local helper. Download this task selection for its input file.']]);
    const download = document.createElement('button'); download.type = 'button'; download.textContent = 'Download task selection';
    download.onclick = () => {
      const blob = new Blob([JSON.stringify(scope, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob); const a = document.createElement('a'); a.href = url; a.download = `task-${task.id}.json`; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    }; helper.append(download);
    for (const d of $('task-detail').querySelectorAll('details')) d.open = opened.has(d.dataset.taskEvidence);
    controls();
  }
  async function loadSelected() {
    if (!selectedId) return;
    if (busy) { reloadRequested = true; return; }
    busy = true;
    const scope = { ...getScope(), taskId: selectedId }; const ticket = ++detailQuery;
    const current = () => ticket === detailQuery && scopeKey(scope) === owner && scope.taskId === selectedId;
    try {
      const [taskResult, execution] = await Promise.all([api('GetTask', scope), api('GetTaskExecution', scope)]);
      if (!current()) return;
      const preparation = execution.preparation;
      let run;
      if (preparation?.attemptId) {
        try { run = (await api('GetTaskAttempt', { ...scope, attemptId: preparation.attemptId })).attempt; }
        catch (error) { if (error.reason !== 'ERROR_REASON_RESOURCE_NOT_FOUND') throw error; }
      }
      if (!current()) return;
      attempt = run;
      recovery.select(run ? { ...scope, attemptId: run.attemptId } : undefined);
      if (preview && (!taskActions(run).reviewPublication || preview.request.expectedPatchDigest !== run.patchDigest || preview.request.expectedQualityDigest !== run.qualityDigest)) resetReview();
      render(taskResult.task, preparation, run);
      $('task-updated').textContent = `Last refreshed ${new Date().toLocaleTimeString()}`;
    } catch (error) {
      if (current()) { attempt = undefined; resetReview(); $('task-detail').replaceChildren(); $('task-status').textContent = 'Task refresh failed. Retry to load current state.'; showError(error); }
    } finally { busy = false; controls(); if (reloadRequested) { reloadRequested = false; loadSelected(); } else schedule(); }
  }
  function schedule() {
    clearTimeout(timer);
    if (!disposed && selectedId && $('task-auto-refresh').checked) timer = setTimeout(() => {
      if (document.visibilityState === 'visible') loadSelected(); else schedule();
    }, 5000);
  }
  async function list(more = false) {
    if (!getScope().conversationId) return;
    const scope = { ...getScope() }; const ticket = ++listQuery;
    const page = await api('ListTasks', { ...scope, afterId: more ? next : '', limit: 20 });
    if (scopeKey(scope) !== owner || ticket !== listQuery) return;
    if (!more) $('task-list').replaceChildren();
    for (const task of page.tasks || []) {
      const button = document.createElement('button'); button.type = 'button'; button.dataset.inspectTask = task.id;
      button.textContent = `${task.proposedTaskKey}: ${task.specification.objective}`;
      button.setAttribute('aria-current', String(task.id === selectedId));
      button.onclick = () => open(task.id).catch(showError); $('task-list').append(button);
    }
    next = page.nextAfterId || ''; $('task-more').hidden = !next;
    $('task-list-status').textContent = $('task-list').children.length ? 'Select a task to inspect its current attempt.' : 'No admitted tasks in this conversation. Approve a reviewed plan to admit work.';
  }
  async function open(id) {
    recovery.select();
    detailQuery++; resetReview(); attempt = undefined; selectedId = id; store(selectionKey(), id);
    $('task-detail').replaceChildren(); $('task-status').textContent = 'Loading task…'; $('task-updated').textContent = '';
    for (const button of $('task-list').children) button.setAttribute('aria-current', String(button.dataset.inspectTask === id));
    // A previous selection may still have a request in flight. Queue this read;
    // the old response cannot render or authorize the previous task.
    await loadSelected();
  }
  async function review() {
    resetReview(); const scope = { ...getScope(), taskId: selectedId, attemptId: attempt?.attemptId }; const ticket = reviewing;
    const result = await api('ReviewTaskPublication', scope);
    if (ticket !== reviewing || scopeKey(scope) !== owner || scope.taskId !== selectedId || scope.attemptId !== attempt?.attemptId) return;
    publicationCommand(scope, selectedId, attempt, result); preview = result;
    const panel = $('task-publication-review');
    const title = document.createElement('h3'); title.textContent = 'Review exact draft publication'; panel.append(title);
    const dl = document.createElement('dl');
    for (const [label, value] of [['Repository', result.repository], ['Source reference', result.source_ref], ['Source commit', result.source_commit], ['Branch', result.branch], ['Base branch', result.base_branch], ['Title', result.title], ['PR body', result.body], ['Commit author', `${result.author_name} <${result.author_email}>`], ['Commit message', result.commit_message], ['Patch digest', result.request.expectedPatchDigest], ['Quality digest', result.request.expectedQualityDigest]]) row(dl, label, value);
    panel.append(dl);
    const label = document.createElement('label'); const consent = document.createElement('input'); consent.type = 'checkbox'; consent.id = 'task-publication-consent';
    label.append(consent, document.createTextNode(' I authorize this exact patch and quality evidence to be pushed and published as a draft with the repository, branch, title and body shown above.'));
    const button = document.createElement('button'); button.type = 'button'; button.textContent = 'Authorize draft publication'; button.dataset.taskAction = 'StartTaskPublication'; button.disabled = true;
    consent.onchange = controls;
    button.onclick = () => { if (!consent.checked || !preview) return; try { const command = publicationCommand(getScope(), selectedId, attempt, preview); submit(command.method, command.body); } catch (error) { resetReview(); showError(error); } };
    panel.append(label, button); controls();
  }
  function select() {
    const key = scopeKey(getScope()); if (key === owner) return;
    recovery.select();
    owner = key; listQuery++; detailQuery++; clearTimeout(timer); resetReview(); attempt = undefined;
    selectedId = saved(selectionKey()) || ''; next = '';
    for (const id of ['task-list', 'task-detail']) $(id).replaceChildren();
    $('task-updated').textContent = ''; $('task-more').hidden = true;
    $('task-status').textContent = ''; $('task-list-status').textContent = getScope().conversationId ? 'Loading admitted tasks…' : 'Choose a conversation.';
    if (getScope().conversationId) { list().catch(showError); if (selectedId) loadSelected(); }
    controls();
  }
  function accepted(command, result) {
    if (command.method === 'RecordTaskRecovery') { recovery.accepted(command); return true; }
    if (!['CancelTaskExecution', 'StartTaskPublication', 'CancelTaskPublication'].includes(command.method)) return false;
    if (scopeKey(command.body) === owner && command.body.taskId === selectedId) {
      resetReview(); $('task-status').textContent = `Command accepted · ${result.receipt?.commandId || 'receipt retained'}. Refreshing current state…`;
      loadSelected();
    }
    return true;
  }
  $('task-refresh').onclick = () => { list().catch(showError); loadSelected(); };
  $('task-more').onclick = () => list(true).catch(showError);
  $('task-auto-refresh').onchange = schedule;
  window.addEventListener('pagehide', () => { disposed = true; clearTimeout(timer); });
  window.addEventListener('pageshow', () => { disposed = false; schedule(); });
  return { select, open, accepted, refreshList: () => list().catch(showError) };
}
