import { scopeKey } from './state.mjs';
export const key = scope => scope ? `${scopeKey(scope)}/${scope.taskId}/${scope.attemptId}` : '';

// Reads discard late results after navigation; mutations use the shared retained command key.
export function createRecoveryHistory({ api, changed = () => {}, failed = () => {} }) {
  let selected; let version = 0; let records = []; let next = ''; let confirmations = new Map(); let answers = new Map(); let rendered = '';
  function select(scope) { selected = scope && { ...scope }; version++; records = []; next = ''; confirmations = new Map(); answers = new Map(); rendered = ''; }
  async function load(more = false) {
    if (!selected || (more && !next)) return;
    const scope = { ...selected }; const query = ++version;
    const current = () => query === version && key(scope) === key(selected);
    try {
      let page = await api('ListTaskRecovery', { ...scope, afterId: more ? next : '', limit: 20 });
      if (!current()) return;
      // Refresh the already reviewed prefix so background reads preserve pagination and drafts.
      const cursors = new Set();
      while (!more && page.nextAfterId && (page.records || []).length < records.length) {
        if (cursors.has(page.nextAfterId)) throw new Error('Recovery pagination did not advance.');
        cursors.add(page.nextAfterId);
        const following = await api('ListTaskRecovery', { ...scope, afterId: page.nextAfterId, limit: 20 });
        if (!current()) return;
        page = { ...following, records: [...(page.records || []), ...(following.records || [])], questionAnswers: [...(page.questionAnswers || []), ...(following.questionAnswers || [])] };
      }
      if ([...(page.records || []), ...(page.questionAnswers || [])].some(r => key(r) !== key(scope))) throw new Error('Recovery history did not match the selected attempt.');
      const unique = new Map((more ? records : []).map(r => [r.id, r]));
      for (const r of page.records || []) unique.set(r.id, r);
      if ((page.confirmations || []).some(c => c.parentAttemptId !== scope.attemptId)) throw new Error('Confirmation did not match the selected attempt.');
      if (!more) { confirmations = new Map(); answers = new Map(); }
      for (const c of page.confirmations || []) confirmations.set(c.proposalId, c);
      for (const a of page.questionAnswers || []) answers.set(a.answer.questionId, a);
      records = [...unique.values()]; next = page.nextAfterId || '';
      const result = { records, next, confirmations: [...confirmations.values()], answers: [...answers.values()] };
      const signature = JSON.stringify(result);
      if (signature !== rendered) { rendered = signature; changed(result); }
    } catch (error) { if (current()) { records = []; next = ''; rendered = ''; changed({ records, next, error: true }); failed(error); } }
  }
  return { select, load };
}
export function confirmationCommand(scope, record, consent, uncertainty) {
  if (!consent || !record?.proposal || key(scope) !== key(record)) throw new Error('Review the exact proposal in its originating attempt before confirming.');
  if (record.proposal.evidence?.providerUncertain && !uncertainty) throw new Error('Acknowledge the possible earlier provider charge before confirming.');
  return { method: 'ConfirmTaskRecovery', body: { workspaceId: scope.workspaceId, projectId: scope.projectId, conversationId: scope.conversationId, taskId: scope.taskId, proposalId: record.id, proposalDigest: record.contentDigest, acknowledgeProviderUncertainty: !!uncertainty } };
}
export function mountRecovery({ api, submit, showError, controls, openAttempt }) {
  const panel = document.getElementById('task-recovery');
  let selected; let version = 0; let list; let more; let composer; let consumed = false;
  function node(tag, text, parent = panel) { const n = document.createElement(tag); n.textContent = text; parent.append(n); return n; }
  function form(parent, label, send) {
    const form = node('form', '', parent); const title = node('label', label, form);
    const input = node('textarea', '', title); input.required = true; input.maxLength = 4096;
    const button = node('button', label, form); button.type = 'submit'; button.dataset.taskAction = 'RecordTaskRecovery';
    const origin = { ...selected };
    form.onsubmit = event => { event.preventDefault(); if (key(origin) === key(selected) && form.reportValidity()) send(input.value, origin); };
    return form;
  }
  const history = createRecoveryHistory({ api, failed: showError, changed({ records, next, confirmations = [], answers = [], error }) {
    list.replaceChildren(); more.hidden = !next; consumed = confirmations.length > 0;
    for (const form of composer.querySelectorAll('[data-recovery-proposal-form]')) form.hidden = consumed;
    if (error) node('p', 'Recovery history could not be loaded. Refresh to try again.', list);
    const answered = new Map([...records, ...answers].filter(r => r.answer).map(r => [r.answer.questionId, r]));
    for (const r of records) {
      const article = node('article', '', list); article.dataset.recoveryId = r.id;
      node('h4', r.proposal ? 'Recovery proposal' : r.question ? 'Operator question' : 'Operator answer', article);
      if (r.proposal) {
        const p = r.proposal; const e = p.evidence || {};
        node('p', p.explanation, article);
        node('p', `Proposed action: ${p.action.replaceAll('_', ' ')}. Additional allowance: ${p.additionalSeconds} seconds and ${p.additionalTokens} tokens.`, article);
        node('p', e.patchAvailable ? 'Accepted changes are available in the task evidence above.' : 'No accepted changes are available. Unreported worker files cannot be assumed reusable.', article);
        node('p', `Quality: ${e.qualityState || 'no retained evidence'}. ${e.limitations || ''}`, article);
        if (e.providerUncertain) node('p', 'An earlier AI request may have been sent and charged. This proposal does not retry or clear that uncertainty.', article);
        const receipt = confirmations.find(c => c.parentAttemptId === r.attemptId);
        if (receipt) {
          node('p', receipt.proposalId === r.id ? `Confirmed once · linked attempt ${receipt.attemptId}.` : `Another proposal was confirmed for this attempt. Linked attempt ${receipt.attemptId}.`, article);
          const open = node('button', 'Inspect continued attempt', article); open.type = 'button'; open.onclick = () => openAttempt(receipt.attemptId);
        } else {
          const origin = { ...selected };
          const label = node('label', ` Authorize this exact ${p.action.replaceAll('_', ' ')} proposal once, with ${p.additionalSeconds} seconds and ${p.additionalTokens} tokens in a linked attempt. ${p.action === 'replace_attempt' ? 'The new attempt starts from the original source; saved changes remain in this parent attempt.' : 'Only retained accepted changes will be reused; unreported worker files are unavailable.'}`, article);
          const consent = document.createElement('input'); consent.type = 'checkbox'; consent.dataset.recoveryConsent = ''; label.prepend(consent); consent.onchange = controls;
          let uncertainty;
          if (e.providerUncertain) {
            const l = node('label', ' I acknowledge the earlier request may have been charged. This authorizes a new bounded request and does not retry or settle the earlier request.', article);
            uncertainty = document.createElement('input'); uncertainty.type = 'checkbox'; uncertainty.dataset.recoveryUncertainty = ''; l.prepend(uncertainty); uncertainty.onchange = controls;
          }
          const confirm = node('button', 'Confirm recovery', article); confirm.type = 'button'; confirm.dataset.taskAction = 'ConfirmTaskRecovery';
          confirm.onclick = () => { try { if (key(origin) !== key(selected)) return; const command = confirmationCommand(origin, r, consent.checked, uncertainty?.checked); submit(command.method, command.body); } catch (error) { showError(error); } };
        }
        const details = node('details' , '', article); node('summary', 'Frozen evidence and proposal identity', details);
        node('pre', JSON.stringify({ id: r.id, digest: r.contentDigest, ...p }, null, 2), details);
      } else if (r.question) {
        node('p', r.question.text, article);
        if (answered.has(r.id)) { node('p', `Answer: ${answered.get(r.id).answer.text}`, article); node('small', 'Direction retained. This answer does not authorize execution or enter provider context.', article); }
        else form(article, 'Record answer', (text, origin) => submit('RecordTaskRecovery', { ...origin, answer: { questionId: r.id, text } }));
      } else if (r.answer) { node('p', r.answer.text, article); node('small', `Answer to question ${r.answer.questionId}`, article); }
      node('small', `Retained ${r.id} · ${r.author || 'local-operator'}`, article);
    }
    if (!records.length && !error) node('p', 'No recovery review records for this attempt.', list);
    controls();
  } });
  async function feedback(scope, query) {
    const items = []; let afterId = '';
    do {
      const page = await api('ListTaskFeedback', { workspaceId: scope.workspaceId, projectId: scope.projectId, conversationId: scope.conversationId, afterId, limit: 32 });
      if (version !== query || key(scope) !== key(selected)) return;
      items.push(...(page.feedback || []).filter(f => f.taskId === scope.taskId && f.attemptId === scope.attemptId)); afterId = page.nextAfterId || '';
    } while (afterId);
    if (!items.length) { node('p', 'Recovery proposals require retained execution or quality feedback.', composer); return; }
    const formElement = form(composer, 'Record recovery proposal', (explanation, origin) => submit('RecordTaskRecovery', { ...origin, proposal: {
      feedbackId: reasons.value, explanation, action: actions.value,
      additionalSeconds: Number(seconds.value), additionalTokens: Number(tokens.value),
    } }));
    formElement.dataset.recoveryProposalForm = ''; formElement.hidden = consumed;
    const reasonLabel = node('label', 'Reason for recovery', formElement); const reasons = node('select', '', reasonLabel);
    for (const f of items) { const o = node('option', f.summary, reasons); o.value = f.id; }
    const actionLabel = node('label', 'Proposed action', formElement); const actions = node('select', '', actionLabel);
    for (const [value, label] of [['continue', 'Continue using saved work'], ['repair_quality', 'Repair failed quality'], ['replace_attempt', 'Explicitly link a replacement attempt']]) { const o = node('option', label, actions); o.value = value; }
    function allowance(label, max) { const l = node('label', label, formElement); const n = node('input', '', l); n.type = 'number'; n.min = '1'; n.max = String(max); n.required = true; return n; }
    const seconds = allowance('Additional seconds', 86400); const tokens = allowance('Additional tokens', 1000000);
    controls();
  }
  function select(scope) {
    if (key(scope) === key(selected)) return false;
    selected = scope && { ...scope }; consumed = false; const query = ++version; history.select(scope); panel.replaceChildren(); panel.hidden = !scope;
    if (!scope) return;
    node('h3', 'Recovery review'); node('p', 'Review saved work and record an exact proposal. Recording proposals and answers does not start work. Only confirming an exact proposal authorizes one linked attempt with the additional allowance shown. Confirmation resolves the parent attention decision; continued work has its own outcome. Operator answers are retained for review and are not supplied to the coding worker.');
    const refresh = node('button', 'Refresh recovery records'); refresh.type = 'button'; refresh.onclick = () => history.load();
    list = node('div', 'Loading recovery records…'); more = node('button', 'Load more recovery records'); more.type = 'button'; more.hidden = true; more.onclick = () => history.load(true);
    composer = node('div', '');
    form(composer, 'Record operator question', (text, origin) => submit('RecordTaskRecovery', { ...origin, question: { text } }));
    history.load(); feedback({ ...scope }, query).catch(error => { if (version === query) showError(error); }); controls(); return true;
  }
  function accepted(command) { if (key(command.body) === key(selected) || (command.method === 'ConfirmTaskRecovery' && selected && scopeKey(command.body) === scopeKey(selected) && command.body.taskId === selected.taskId)) history.load(); }
  return { select, accepted, refresh: () => history.load() };
}
