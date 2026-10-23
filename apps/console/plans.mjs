// Local edits and immutable reviewed versions have independent state.
import { scopeKey } from './state.mjs';
const $ = id => document.getElementById(id);
const lines = value => value.split('\n').map(s => s.trim()).filter(Boolean);
const blank = () => ({ title: '', objective: '', scope: '', outOfScope: '', constraints: [], unresolvedQuestions: [], tasks: [] });
export function planDraftKey(scope) { return `plan-draft:${scopeKey(scope)}`; }
export function planCommand(scope, draft) {
  const { planId, expectedVersionId, content } = draft;
  return planId ? { method: 'RevisePlan', body: { ...scope, planId, expectedVersionId, content } }
    : { method: 'CreatePlan', body: { ...scope, content } };
}

// Approval always binds the immutable version the operator actually reviewed.
export function approvalCommand(scope, version) {
  if (scopeKey(scope) !== scopeKey(version)) throw new Error('The reviewed version belongs to another conversation.');
  return { method: 'ApprovePlan', body: { ...scope, planId: version.planId, expectedVersionId: version.id, contentDigest: version.contentDigest } };
}
export function approvalBlocker(version, head, approval) {
  if (!version) return 'Select a retained plan to review.';
  if (approval || head?.approvalId) return 'Approved. The admitted task set is immutable and awaiting execution.';
  if (head?.currentVersionId !== version.id) return 'A newer version exists. Your reviewed version and local draft remain unchanged. Select the plan to review its current version.';
  if (version.content.unresolvedQuestions?.length) return 'Resolve scope and acceptance questions in a new version before approval.';
  if (!version.content.tasks?.length || version.content.tasks.some(task => !task.acceptanceCriteria?.length)) return 'Approval requires proposed tasks with acceptance criteria.';
  return '';
}

export function mountPlans({ getScope, api, submit, showError, saved, store, controls }) {
  let owner = '';
  let draft = { content: blank() };
  let reviewed;
  let heads = [];
  let approval;
  let approvalQuery = 0;
  let query = 0;
  let historyBefore = '0';
  let historyPlan = '';
  const fields = ['title', 'objective', 'scope', 'outOfScope', 'constraints', 'unresolvedQuestions'];
  const taskFields = ['key', 'objective', 'scope', 'outOfScope', 'acceptanceCriteria', 'repository', 'sourceRef', 'executionLimits', 'unresolvedInputs'];
  const arrays = ['constraints', 'unresolvedQuestions', 'acceptanceCriteria', 'unresolvedInputs'];
  const inputValue = (name, value) => arrays.includes(name) ? lines(value) : value;
  const displayValue = value => Array.isArray(value) ? value.join('\n') : value || '';
  function persist() { store(planDraftKey(getScope()), draft); }
  function readEditor() {
    for (const name of fields) draft.content[name] = inputValue(name, $('plan-editor').querySelector(`:scope > label [name="${name}"]`).value);
    draft.content.tasks = [...$('plan-tasks').children].map(card => Object.fromEntries(taskFields.map(name => [name, inputValue(name, card.querySelector(`[name="${name}"]`).value)])));
    persist();
  }
  function taskEditor(value) {
    const card = document.createElement('fieldset');
    const legend = document.createElement('legend'); legend.textContent = 'Proposed task'; card.append(legend);
    const labels = { key: 'Task key', objective: 'Task objective', scope: 'Included scope', outOfScope: 'Excluded scope', acceptanceCriteria: 'Acceptance criteria · one per line', repository: 'Declared repository', sourceRef: 'Declared source reference', executionLimits: 'Execution limits', unresolvedInputs: 'Unresolved inputs · one per line' };
    for (const name of taskFields) {
      const label = document.createElement('label'); label.textContent = labels[name];
      const input = document.createElement(['key', 'repository', 'sourceRef'].includes(name) ? 'input' : 'textarea');
      input.name = name; input.value = displayValue(value[name]);
      input.maxLength = name === 'key' ? 64 : name === 'repository' ? 1024 : name === 'sourceRef' ? 256 : 4096;
      input.required = ['key', 'objective', 'scope'].includes(name);
      label.append(input); card.append(label);
    }
    const remove = document.createElement('button'); remove.type = 'button'; remove.textContent = 'Remove proposed task';
    remove.onclick = () => { card.remove(); readEditor(); };
    card.append(remove); return card;
  }
  function renderEditor() {
    for (const name of fields) $('plan-editor').querySelector(`:scope > label [name="${name}"]`).value = displayValue(draft.content[name]);
    $('plan-tasks').replaceChildren(...draft.content.tasks.map(taskEditor));
    $('plan-editing').textContent = draft.planId ? `Revising version ${draft.expectedVersionId}. A concurrent edit will require review.` : 'New operator-authored draft.';
    controls();
  }
  function paragraph(parent, label, value) {
    const section = document.createElement('div');
    const title = document.createElement('strong'); title.textContent = label;
    const text = document.createElement('p'); text.textContent = displayValue(value) || 'Not specified';
    section.append(title, text); parent.append(section);
  }
  function showVersion(value) {
    reviewed = value;
    approval = undefined;
    approvalQuery++;
    $('plan-approve-consent').checked = false;
    $('plan-admission').replaceChildren();
    $('task-detail').replaceChildren();
    $('plan-review').replaceChildren();
    const heading = document.createElement('h3'); heading.textContent = `${value.content.title} · version ${value.ordinal}`;
    $('plan-review').append(heading);
    paragraph($('plan-review'), 'Origin', `Project ${value.projectId} · conversation ${value.conversationId}`);
    paragraph($('plan-review'), 'Retained version', `${value.id}\n${value.contentDigest}\nAuthor: ${value.author}`);
    if (value.provenance) {
      const p = value.provenance;
      paragraph($('plan-review'), 'Assumptions', p.assumptions);
      paragraph($('plan-review'), 'Declared evidence · unverified', p.evidence);
      paragraph($('plan-review'), 'Proposal origin', `Turn ${p.sourceTurnId}\nConfiguration ${p.configurationRevisionId}\nContext ${p.contextDigest}`);
      paragraph($('plan-review'), 'Retained provider result', `Call ${p.providerCallId}\nResponse ${p.providerResponseId}\nInvocation ${p.invocationId}`);
      paragraph($('plan-review'), 'Authenticated producer', `Assignment ${p.assignmentId} · generation ${p.executorGeneration}\n${p.reportSource}\nReport ${p.reportId}\n${p.reportDigest}`);
    }
    for (const [name,label] of [['objective','Objective'],['scope','Included scope'],['outOfScope','Excluded scope'],['constraints','Constraints'],['unresolvedQuestions','Unresolved questions']]) paragraph($('plan-review'),label,value.content[name]);
    for (const task of value.content.tasks || []) {
      const article = document.createElement('article'); article.className = 'plan-task';
      paragraph(article, `Proposed task ${task.key}`, task.objective);
      for (const [name,label] of [['scope','Included scope'],['outOfScope','Excluded scope'],['acceptanceCriteria','Acceptance criteria'],['repository','Declared repository · unverified'],['sourceRef','Declared source · unverified'],['executionLimits','Execution limits'],['unresolvedInputs','Unresolved inputs']]) paragraph(article,label,task[name]);
      $('plan-review').append(article);
    }
    reviewStatus();
  }
  async function open(planId, versionId = '') {
    const selected = { ...getScope() }; const ticket = ++query;
    const result = await api('GetPlanVersion', { ...selected, planId, versionId });
    if (ticket !== query || scopeKey(selected) !== owner) return;
    showVersion(result.version);
    await loadApproval();
    if (ticket !== query || scopeKey(selected) !== owner) return;
    if (historyPlan !== planId) { historyPlan = planId; historyBefore = '0'; $('plan-history').replaceChildren(); }
    await history(true);
  }
  async function history(reset = false) {
    if (!reviewed) return;
    const selected = { ...getScope() }; const planId = reviewed.planId;
    const page = await api('ListPlanVersions', { ...selected, planId, beforeOrdinal: reset ? '0' : historyBefore, limit: 10 });
    if (scopeKey(selected) !== owner || reviewed?.planId !== planId) return;
    if (reset) $('plan-history').replaceChildren();
    for (const version of page.versions || []) {
      const button = document.createElement('button'); button.type = 'button'; button.textContent = `Review version ${version.ordinal}`; button.dataset.planVersion = version.id;
      button.onclick = () => open(planId, version.id).catch(showError); $('plan-history').append(button);
    }
    historyBefore = page.nextBeforeOrdinal || '0'; $('plan-history-more').hidden = historyBefore === '0';
  }
  const head = () => heads.find(value => value.id === reviewed?.planId);
  function reviewStatus() {
    const blocker = approvalBlocker(reviewed, head(), approval);
    $('plan-review-status').textContent = blocker || 'Review the exact version and task set, then explicitly approve admission.';
    $('plan-approval-basis').textContent = reviewed ? `Version ${reviewed.id} · ${reviewed.contentDigest} · ${reviewed.content.tasks?.length || 0} proposed tasks` : '';
    controls();
  }
  function renderTask(parent, task) {
    paragraph(parent, `Task ${task.proposedTaskKey} · ${task.state.replaceAll('_', ' ')}`, task.id);
    paragraph(parent, 'Immutable origin', `Workspace ${task.workspaceId}\nProject ${task.projectId}\nOriginating conversation ${task.conversationId}\nPlan ${task.planId}\nVersion ${task.versionId}\n${task.contentDigest}\nApproval ${task.approvalId}`);
    paragraph(parent, 'Prerequisites recorded at admission', task.executionBlockers);
    paragraph(parent, 'Frozen task objective', task.specification.objective);
    for (const [name, label] of [['scope', 'Included scope'], ['outOfScope', 'Excluded scope'], ['acceptanceCriteria', 'Acceptance criteria'], ['repository', 'Declared repository · unverified'], ['sourceRef', 'Declared source · unverified'], ['executionLimits', 'Declared execution limits'], ['unresolvedInputs', 'Unresolved execution inputs']]) paragraph(parent, label, task.specification[name]);
  }
  function showApproval(result) {
    approval = result.approval;
    $('plan-admission').replaceChildren();
    if (approval) {
      paragraph($('plan-admission'), 'Approval receipt', `${approval.id}\nOperator ${approval.operator}\nCommand ${approval.commandId}\nVersion ${approval.versionId}\n${approval.contentDigest}\nApproved ${approval.createdAt}`);
      paragraph($('plan-admission'), 'Admission', `${result.tasks.length} tasks admitted and awaiting execution. Coding work has not been dispatched.`);
      for (const task of result.tasks) {
        const article = document.createElement('article'); article.className = 'plan-task'; article.dataset.taskId = task.id;
        renderTask(article, task);
        const button = document.createElement('button'); button.type = 'button'; button.textContent = `Refresh task ${task.proposedTaskKey}`;
        button.onclick = async () => {
          try {
            const selected = { ...getScope() }; const ticket = query;
            const response = await api('GetTask', { ...selected, taskId: task.id });
            if (ticket !== query || scopeKey(selected) !== owner) return;
            const execution = await api('GetTaskExecution', { ...selected, taskId: task.id });
            if (ticket !== query || scopeKey(selected) !== owner) return;
            $('task-detail').replaceChildren(); renderTask($('task-detail'), response.task);
            const preparation = execution.preparation;
            if (preparation) {
              paragraph($('task-detail'), 'Execution preparation', preparation.attemptId ? `Prepared attempt ${preparation.attemptId}.` : 'Blocked');
              paragraph($('task-detail'), 'Preparation blockers', preparation.blockers);
              paragraph($('task-detail'), 'Frozen execution inputs', JSON.stringify(preparation.inputs, null, 2));
              paragraph($('task-detail'), 'Frozen configuration', preparation.configurationRevisionId);
              if (preparation.attemptId) {
                let attempt;
                try { attempt = (await api('GetTaskAttempt', { ...selected, taskId: task.id, attemptId: preparation.attemptId })).attempt; }
                catch (error) { if (error.reason !== 'ERROR_REASON_RESOURCE_NOT_FOUND') throw error; }
                if (ticket !== query || scopeKey(selected) !== owner) return;
                paragraph($('task-detail'), 'Execution state', attempt?.state || 'Prepared; not started');
                if (attempt) {
                  paragraph($('task-detail'), 'Worker generation and provider state', `${attempt.generation} · ${attempt.providerState}`);
                  paragraph($('task-detail'), 'Retained patch', attempt.patch || 'No accepted changes yet');
                  paragraph($('task-detail'), 'Publication', attempt.publication ? JSON.stringify(attempt.publication, null, 2) : 'Not authorized');
                  paragraph($('task-detail'), 'Coding outcome', attempt.executionOutcome || 'Pending');
                  paragraph($('task-detail'), 'Quality outcome', attempt.quality ? `${attempt.quality.state} · ${attempt.quality.safeReason || 'Ayni evidence retained'}` : 'Pending');
                  if (attempt.quality) paragraph($('task-detail'), 'Quality evidence', JSON.stringify(attempt.quality, null, 2));
                  if (attempt.safeReason) paragraph($('task-detail'), 'Execution outcome', attempt.safeReason);
                  if (['queued', 'claimed', 'reporting'].includes(attempt.state)) {
                    const cancel = document.createElement('button'); cancel.type = 'button'; cancel.textContent = 'Cancel task execution'; cancel.dataset.cancelTask = task.id;
                    cancel.onclick = () => submit('CancelTaskExecution', { ...selected, taskId: task.id, attemptId: preparation.attemptId });
                    $('task-detail').append(cancel); controls();
                  }
                }
              }
            }
          } catch (error) { showError(error); }
        };
        article.append(button); $('plan-admission').append(article);
      }
    }
    reviewStatus();
  }
  async function loadApproval() {
    if (!reviewed) return;
    const selected = { ...getScope() }; const planId = reviewed.planId; const ticket = ++approvalQuery;
    const result = await api('GetPlanApproval', { ...selected, planId });
    if (ticket !== approvalQuery || scopeKey(selected) !== owner || reviewed?.planId !== planId) return;
    showApproval(result);
  }
  function update(plans = []) {
    heads = plans;
    $('plan-list').replaceChildren();
    for (const plan of heads) {
      const button = document.createElement('button'); button.type = 'button'; button.textContent = `${plan.title} · version ${plan.currentOrdinal}${plan.approvalId ? ' · approved' : ''}`; button.dataset.planId = plan.id;
      button.onclick = () => open(plan.id).catch(showError); $('plan-list').append(button);
    }
    reviewStatus();
    if (reviewed && head()?.approvalId && head().approvalId !== approval?.id) loadApproval().catch(showError);
  }
  function select() {
    const next = scopeKey(getScope());
    if (next === owner) return;
    owner = next; query++; approvalQuery++; approval = undefined; reviewed = undefined; heads = []; historyPlan = ''; historyBefore = '0';
    draft = saved(planDraftKey(getScope())) || { content: blank() };
    $('plan-review').replaceChildren(); $('plan-history').replaceChildren(); $('plan-list').replaceChildren();
    $('plan-admission').replaceChildren(); $('task-detail').replaceChildren(); $('plan-approve-consent').checked = false; $('plan-approval-basis').textContent = '';
    $('plan-history-more').hidden = true; $('plan-review-status').textContent = 'Select a retained plan to review.';
    renderEditor();
  }
  function accepted(command, result) {
    if (command.method === 'CancelTaskExecution') { if (scopeKey(command.body) === owner) paragraph($('task-detail'), 'Cancellation requested', 'The worker is fenced. Refresh the task to inspect its retained state.'); return; }
    if (command.method === 'ApprovePlan') {
      if (scopeKey(command.body) === owner && reviewed?.planId === command.body.planId) { approvalQuery++; showApproval(result); }
      if (scopeKey(command.body) === owner) api('ListPlans', getScope()).then(value => { if (scopeKey(command.body) === owner) update(value.plans); }).catch(showError);
      return;
    }
    // Only clear the draft belonging to the exact accepted command. Another scope may be open.
    const key = planDraftKey(command.body); store(key, null);
    if (scopeKey(command.body) !== owner) return;
    draft = { content: blank() }; renderEditor(); showVersion(result.version);
    api('ListPlans', getScope()).then(result => { if (planDraftKey(command.body) === planDraftKey(getScope())) update(result.plans); }).catch(showError);
    historyPlan = result.version.planId; historyBefore = '0'; history(true).catch(showError);
  }
  $('plan-editor').oninput = readEditor;
  $('plan-editor').onsubmit = event => { event.preventDefault(); readEditor(); const command = planCommand(getScope(), draft); submit(command.method, command.body); };
  $('plan-add-task').onclick = () => { if (draft.content.tasks.length >= 32) { showError(new Error('A plan can contain at most 32 proposed tasks.')); return; } $('plan-tasks').append(taskEditor({})); readEditor(); controls(); };
  $('plan-discard').onclick = () => { draft = { content: blank() }; store(planDraftKey(getScope()), null); renderEditor(); };
  $('plan-edit').onclick = () => {
    if (!reviewed) return;
    if (draft.content.title || draft.content.objective || draft.content.scope || draft.content.tasks.length) { showError(new Error('Save or discard the local draft before editing a retained version.')); return; }
    if (heads.find(h => h.id === reviewed.planId)?.currentVersionId !== reviewed.id) { showError(new Error('Review the current version before revising this plan.')); return; }
    draft = { planId: reviewed.planId, expectedVersionId: reviewed.id, content: structuredClone(reviewed.content) };
    draft.content.tasks ||= []; draft.content.constraints ||= []; draft.content.unresolvedQuestions ||= [];
    persist(); renderEditor(); $('plan-editor-details').open = true;
  };
  $('plan-approve-consent').onchange = controls;
  $('plan-approve').onclick = () => {
    if (!$('plan-approve-consent').checked || approvalBlocker(reviewed, head(), approval)) return;
    const command = approvalCommand(getScope(), reviewed);
    submit(command.method, command.body);
  };
  $('plan-history-more').onclick = () => history().catch(showError);
  return { select, update, accepted, canEdit: () => !!reviewed && head()?.currentVersionId === reviewed.id && !approval && !head()?.approvalId, canApprove: () => !!reviewed && !approvalBlocker(reviewed, head(), approval) && $('plan-approve-consent').checked };
}
