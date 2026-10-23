import { mountTasks } from './tasks.mjs';
import { mountFeedback } from './feedback.mjs';
import { mountPlans } from './plans.mjs';
import { newerOrEqual, describe, scopeKey, makeCommand, appendActivity, providerCallView } from './state.mjs';

const $ = id => document.getElementById(id);
const saved = key => { try { return JSON.parse(sessionStorage.getItem(key)); } catch { return null; } };
const store = (key, value) => { try { if (value == null) sessionStorage.removeItem(key); else sessionStorage.setItem(key, JSON.stringify(value)); } catch { /* In-memory retries remain available when storage is disabled. */ } };
let scope = saved('selection') || {};
let pending = saved('pending');
let sending = false;
let snapshot;
let cursor = '';
let source;
let paused = false;
let generation = 0;
let activities = [];
let historyBefore = '0';
let historySnapshotId = '';
let historyHasMore = true;
let historyIds = new Set();
let conversationAfter = '';
let workspaceAfter = '';
let projectAfter = '';
let discoveryGeneration = 0;
let discoveryBusy = false;
let chain = Promise.resolve();
let queuedFrames = 0;
let configuration;
let currentSession;
let providerQuery = 0;
const tasks = mountTasks({ getScope: scopeBody, api, submit, showError, saved, store, controls });
const plans = mountPlans({ getScope: scopeBody, api, submit, showError, saved, store, controls, tasks });
const feedback = mountFeedback({ getScope: scopeBody, api, showError, openTask: async (origin, taskId, fromProject) => {
  if (fromProject || scope.conversationId !== origin.conversationId) await selectConversation(origin.conversationId);
  if (scopeKey(scope) !== scopeKey(origin)) return;
  await tasks.open(taskId);
  $(fromProject ? 'conversation-feedback' : 'task-detail').scrollIntoView({ block: 'start', behavior: 'smooth' });
} });

function showError(error) { $('error').textContent = error.message || String(error); $('error').hidden = false; }
function clearError() { $('error').hidden = true; }
async function api(method, body) {
  const response = await fetch(`/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body), signal: AbortSignal.timeout(15000) });
  const result = await response.json();
  if (!response.ok) { const error = new Error(`${result.reason || result.code || 'Request failed'}${result.requestId ? ` · ${result.requestId}` : ''}`); error.reason = result.reason; error.code = result.code; throw error; }
  return result;
}
function scopeBody() { return { workspaceId: scope.workspaceId, projectId: scope.projectId, conversationId: scope.conversationId }; }
function controls() {
  for (const form of document.querySelectorAll('form')) {
    const unavailable = !!pending || sending || discoveryBusy || (form.id === 'agent-configuration' && (!configuration || configuration.workspaceId !== scope.workspaceId)) || (form.id === 'create-project' && !scope.workspaceId) || (form.id === 'create-conversation' && !scope.projectId) || (form.id === 'send' && !scope.conversationId) || (form.id === 'plan-editor' && !scope.conversationId);
    for (const input of form.elements) input.disabled = unavailable;
  }
  $('plan-approve').disabled = !plans.canApprove() || !!pending || sending || discoveryBusy;
  $('plan-approve-consent').disabled = !!pending || sending || discoveryBusy;
  $('plan-edit').disabled = !plans.canEdit() || !!pending || sending || discoveryBusy;
  $('pending').hidden = !pending;
  $('retry').disabled = sending;
  $('pending-description').textContent = pending ? `${pending.method}: retry uses the original content, scope, and command key.` : '';
  $('toggle-watch').disabled = !scope.conversationId;
  $('refresh-session').disabled = !scope.conversationId || sending;
  $('replace-session').disabled = !currentSession?.canReplace || !!pending || sending || discoveryBusy;
  $('refresh-provider-calls').disabled = !scope.conversationId || sending;
  $('task-refresh').disabled = !scope.conversationId;
  for (const button of document.querySelectorAll('[data-task-action]')) button.disabled = !!pending || sending || discoveryBusy || (button.dataset.taskAction === 'StartTaskPublication' && !$('task-publication-consent')?.checked);
  for (const button of document.querySelectorAll('[data-cancel-turn]')) button.disabled = !!pending || sending || discoveryBusy;
  for (const button of document.querySelectorAll('[data-acknowledge-call]')) button.disabled = !!pending || sending || discoveryBusy || !button.closest('article').querySelector('input[type=checkbox]')?.checked;
}
async function submit(method, body) {
  if (pending || sending) return;
  pending = makeCommand(method, body);
  store('pending', pending);
  await retry();
}
async function retry() {
  if (!pending || sending) return;
  sending = true; controls(); clearError();
  try {
    const command = pending;
    const result = await api(command.method, command.body);
    pending = null; store('pending', null);
    $('receipt').textContent = result.receipt?.commandId || result.approval?.commandId || result.record?.id || 'Accepted';
    if (tasks.accepted(command, result)) {
      // Task receipts share the exact pending-command recovery path.
    } else if (['CreatePlan', 'RevisePlan', 'ApprovePlan'].includes(command.method)) {
      plans.accepted(command, result);
    } else if (command.method === 'SendMessage') {
      if (scopeKey(command.body) === scopeKey(scope)) $('content').value = '';
    } else if (command.method === 'UpdateAgentConfiguration') {
      if (command.body.configuration.workspaceId === scope.workspaceId) await loadConfiguration();
    } else if (command.method === 'CancelTurn') {
      if (scopeKey(command.body) === scopeKey(scope)) {
        $('receipt').textContent = `${result.commandId || 'Accepted'} · ${result.state || 'cancellation requested'}`;
        const selected = scopeBody();
        const current = await api('GetConversationSnapshot', selected);
        if (scopeKey(selected) === scopeKey(scope)) { renderSnapshot(current.snapshot); await loadProviderCalls(); }
      }
    } else if (command.method === 'AcknowledgeProviderUncertainty') {
      if (scopeKey(command.body) === scopeKey(scope)) {
        $('receipt').textContent = `${result.commandId || 'Accepted'} · uncertainty acknowledged`;
        await loadProviderCalls(); await loadSession();
      }
    } else if (command.method === 'ReplaceConversationSession') {
      if (scopeKey(command.body) === scopeKey(scope)) await loadSession();
    } else {
      if (result.workspace) scope = { workspaceId: result.workspace.id };
      if (result.project) scope = { workspaceId: result.project.workspaceId, projectId: result.project.id };
      if (result.conversation) scope = { workspaceId: result.conversation.workspaceId, projectId: result.conversation.projectId, conversationId: result.conversation.id };
      store('selection', scope); await loadDiscovery(); await selectConversation(scope.conversationId);
    }
  } catch (error) {
    // Canonical validation/scope/conflict rejections did not admit this command.
    if (['InvalidArgument', 'NotFound', 'AlreadyExists', 'Aborted', 'FailedPrecondition'].includes(error.code) || (error.code === 'ResourceExhausted' && error.reason === 'ERROR_REASON_BOUND_EXCEEDED')) { pending = null; store('pending', null); }
    showError(error);
  }
  finally { sending = false; controls(); }
}
function addOption(select, value, label) { const option = document.createElement('option'); option.value = value; option.textContent = label; select.append(option); }
function resetSelect(id, placeholder) { $(id).replaceChildren(); addOption($(id), '', placeholder); }
function addPageOption(id, next) { if (next) addOption($(id), '__more__', 'Load more…'); }
async function workspacePage(after = '') {
  const page = await api('ListWorkspaces', { afterId: after, limit: 100 });
  for (const value of page.workspaces || []) addOption($('workspace'), value.id, value.displayName);
  workspaceAfter = page.nextAfterId || ''; addPageOption('workspace', workspaceAfter);
}
async function projectPage(after = '') {
  const page = await api('ListProjects', { workspaceId: scope.workspaceId, afterId: after, limit: 100 });
  for (const value of page.projects || []) addOption($('project'), value.id, value.displayName);
  projectAfter = page.nextAfterId || ''; addPageOption('project', projectAfter);
}
async function conversationPage(after = '') {
  const version = discoveryGeneration;
  const parent = { workspaceId: scope.workspaceId, projectId: scope.projectId };
  const page = await api('ListConversations', { ...parent, afterId: after, limit: 100 });
  if (version !== discoveryGeneration) return;
  for (const value of page.conversations || []) {
    const button = document.createElement('button'); button.textContent = value.title; button.dataset.id = value.id;
    button.setAttribute('aria-current', String(scope.conversationId === value.id));
    button.onclick = () => selectConversation(value.id).catch(showError); $('conversations').append(button);
  }
  conversationAfter = page.nextAfterId || ''; $('more-conversations').hidden = !conversationAfter;
}
async function loadDiscovery() {
  discoveryBusy = true; controls();
  for (const id of ['workspace', 'project', 'refresh']) $(id).disabled = true;
  try {
  discoveryGeneration++;
  resetSelect('workspace', 'Choose a workspace'); resetSelect('project', 'Choose a project'); $('conversations').replaceChildren();
  await workspacePage();
  // A saved selection may be beyond the first page. Load only until it is found.
  while (scope.workspaceId && !$('workspace').querySelector(`option[value="${CSS.escape(scope.workspaceId)}"]`) && workspaceAfter) { $('workspace').lastElementChild.remove(); await workspacePage(workspaceAfter); }
  $('workspace').value = scope.workspaceId || '';
  await loadConfiguration();
  if (scope.workspaceId) {
    await projectPage();
    while (scope.projectId && !$('project').querySelector(`option[value="${CSS.escape(scope.projectId)}"]`) && projectAfter) { $('project').lastElementChild.remove(); await projectPage(projectAfter); }
    $('project').value = scope.projectId || '';
  }
  if (scope.projectId) {
    await conversationPage();
    while (scope.conversationId && ![...$('conversations').children].some(button => button.dataset.id === scope.conversationId) && conversationAfter) await conversationPage(conversationAfter);
  }
  controls();
  } finally { discoveryBusy = false; controls(); for (const id of ['workspace', 'project', 'refresh']) $(id).disabled = false; }
}
async function loadConfiguration() {
  configuration = undefined;
  const workspaceId = scope.workspaceId;
  $('configuration-status').textContent = workspaceId ? 'Loading configuration…' : 'Choose a workspace.';
  $('agent-configuration').reset(); controls();
  if (!workspaceId) return;
  const result = await api('GetAgentConfiguration', { workspaceId });
  if (scope.workspaceId !== workspaceId) return;
  configuration = result.configuration;
  for (const key of ['instructions', 'provider', 'model', 'maxInputTokens', 'maxOutputTokens', 'maxTotalTokens', 'idleTimeoutSeconds']) $('agent-configuration').elements.namedItem(key).value = configuration[key] || '';
  $('configuration-status').textContent = `Revision ${configuration.revisionId} · interface v1`;
  controls();
}
async function loadSession() {
  const selected = scopeBody();
  if (!selected.conversationId) return;
  const result = await api('ListConversationSessions', selected);
  if (scopeKey(selected) !== scopeKey(scope)) return;
  currentSession = result.sessions?.[0];
  $('session-state').textContent = currentSession ? `${currentSession.state} · generation ${currentSession.generation} · ${currentSession.id}${!currentSession.canReplace && ['idle', 'expired'].includes(currentSession.state) ? ' · finishing accepted work; refresh to check.' : ''}` : 'Session starts with the first eligible message.';
  controls();
}
async function loadProviderCalls() {
  const selected = scopeBody();
  const query = ++providerQuery;
  if (!selected.conversationId) return;
  const result = await api('ListProviderCalls', selected);
  if (scopeKey(selected) !== scopeKey(scope) || query !== providerQuery) return;
  const calls = result.calls || [];
  $('provider-call-status').textContent = calls.length ? `${calls.length} retained provider call${calls.length === 1 ? '' : 's'}. Usage is reported tokens; missing usage remains unknown.` : 'No retained provider calls. Configuration alone does not show execution.';
  $('provider-calls').replaceChildren();
  for (const call of calls) {
    const view = providerCallView(call);
    const article = document.createElement('article'); article.className = 'provider-call';
    const heading = document.createElement('h3'); heading.textContent = view.binding;
    const state = document.createElement('p'); state.textContent = `${view.state}${call.safeReason ? ` · ${call.safeReason}` : ''}`;
    const usage = document.createElement('p'); usage.textContent = `Reported usage: ${view.usage}`;
    const reservation = document.createElement('p'); reservation.textContent = `Reserved allowance: ${view.reservation}`;
    const identity = document.createElement('p'); identity.className = 'call-identity'; identity.textContent = `Call ${call.callId} · turn ${call.turnId}`;
    article.append(heading, state, usage, reservation, identity);
    if (view.tools.length) { const tools = document.createElement('p'); tools.textContent = `Scoped reads: ${view.tools.join(' · ')}`; article.append(tools); }
    if (call.reportedModel) { const model = document.createElement('p'); model.textContent = `Reported model: ${call.reportedModel}`; article.append(model); }
    if (view.notice) { const notice = document.createElement('p'); notice.className = 'provider-notice'; notice.textContent = view.notice; article.append(notice); }
    if (call.state === 'uncertain' && !call.uncertaintyAcknowledged) {
      const label = document.createElement('label');
      const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.dataset.acknowledgeConsent = call.callId;
      label.append(checkbox, document.createTextNode(' I acknowledge this call may have incurred a charge. Keep its evidence and stop this turn without retrying it.'));
      const button = document.createElement('button'); button.textContent = 'Acknowledge uncertain outcome'; button.dataset.acknowledgeCall = call.callId;
      const acknowledgement = { ...selected, turnId: call.turnId, callId: call.callId, requestDigest: call.requestDigest, acknowledgePossibleCharge: true };
      button.onclick = () => { if (checkbox.checked) submit('AcknowledgeProviderUncertainty', acknowledgement); };
      checkbox.onchange = controls;
      article.append(label, button);
    }
    $('provider-calls').append(article);
  }
  controls();
}
function renderTurnControls(turns) {
  $('turn-controls').replaceChildren();
  for (const turn of turns) {
    const item = document.createElement('li');
    const label = document.createElement('span'); label.textContent = `Turn ${turn.sequence}: ${describe(turn.executionState)}`;
    const button = document.createElement('button'); button.textContent = 'Cancel turn'; button.dataset.cancelTurn = turn.id;
    const selected = { ...scopeBody(), turnId: turn.id };
    button.onclick = () => submit('CancelTurn', selected);
    item.append(label, button); $('turn-controls').append(item);
  }
}
function renderSnapshot(value) {
  if (snapshot && !newerOrEqual(value.conversationRevision, snapshot.conversationRevision)) return;
  snapshot = value;
  plans.update(value.plans);
  $('title').textContent = value.title;
  $('subtitle').textContent = 'Messages and activity retained by the workflow.';
  $('breadcrumb').textContent = `${$('workspace').selectedOptions[0]?.textContent || 'Workspace'} / ${$('project').selectedOptions[0]?.textContent || 'Project'}`;
  $('revision').textContent = value.conversationRevision || '0';
  $('messages').replaceChildren();
  for (const message of value.recentMessages || []) addMessage($('messages'), message.role === 'MESSAGE_ROLE_USER' ? 'You' : 'Assistant', message.content, message.role === 'MESSAGE_ROLE_USER');
  if (!(value.recentMessages || []).length) showEmpty();
  const turns = value.nonterminalTurns || [];
  renderTurnControls(turns);
  $('turn-state').textContent = turns.length ? `${turns.length} active or queued · ${describe(value.currentActivity?.presentationState)}${value.currentActivity?.waitingOn ? ` · waiting on ${describe(value.currentActivity.waitingOn)}` : ''}` : 'All accepted turns have finished.';
  if (value.currentActivity?.safeReason) $('turn-state').textContent += ` · ${value.currentActivity.safeReason}`;
  $('older').hidden = historySnapshotId ? !historyHasMore : !(value.historyCounts?.hasOlderHistory || false);
  controls();
}
function showEmpty() {
  const empty = document.createElement('div'); empty.className = 'empty';
  const heading = document.createElement('h2'); heading.textContent = 'Start with a question.';
  const detail = document.createElement('p'); detail.textContent = 'Messages, activity, and accepted responses will appear here.';
  empty.append(heading, detail); $('messages').append(empty);
}
function addMessage(target, role, content, user = false) {
  const article = document.createElement('article'); article.className = `message${user ? ' user' : ''}`;
  const heading = document.createElement('h3'); heading.textContent = role;
  const text = document.createElement('p'); text.textContent = content || '';
  article.append(heading, text); target.append(article);
}
function closeWatch() { if (source) source.close(); source = null; }
function openWatch(version) {
  closeWatch(); if (paused || !scope.conversationId || version !== generation) return;
  const query = new URLSearchParams(scopeBody()); if (cursor) query.set('afterCursor', cursor);
  source = new EventSource(`/watch?${query}`);
  const current = source;
  current.onopen = () => { if (current === source) $('connection').textContent = 'Connected'; };
  current.onerror = () => { if (current === source) $('connection').textContent = 'Reconnecting…'; };
  current.addEventListener('frame', event => {
    if (current !== source || version !== generation) return;
    if (queuedFrames >= 64) { closeWatch(); $('connection').textContent = 'Catching up…'; setTimeout(() => openWatch(version), 1500); return; }
    queuedFrames++;
    chain = chain.then(async () => {
      if (version !== generation || current !== source) return;
      const frame = JSON.parse(event.data);
      if (frame.snapshot) {
        renderSnapshot(frame.snapshot); await Promise.all([loadProviderCalls(), feedback.refresh()]);
        if (version !== generation || current !== source) return;
        cursor = frame.snapshot.resumeCursor;
      }
      if (frame.update) {
        // Commit the browser cursor only after refreshing the authoritative projection.
        const result = await api('GetConversationSnapshot', scopeBody());
        if (version !== generation || current !== source) return;
        renderSnapshot(result.snapshot);
        await Promise.all([loadProviderCalls(), feedback.refresh()]);
        if (version !== generation || current !== source) return;
        activities = appendActivity(activities, frame.update);
        $('activity').replaceChildren(...activities.map(update => { const li = document.createElement('li'); li.textContent = `${describe(update.presentationState)} · ${update.safeReason || 'workflow update'}`; return li; }));
        cursor = frame.update.cursor;
      }
      $('cursor').textContent = cursor || '—';
    }).catch(error => { if (version === generation && current === source) { showError(error); closeWatch(); $('connection').textContent = 'Reconnecting…'; setTimeout(() => openWatch(version), 1500); } }).finally(() => { queuedFrames--; });
  });
  current.addEventListener('fault', event => {
    if (current !== source) return;
    closeWatch(); const error = JSON.parse(event.data);
    if (error.reason === 'ERROR_REASON_WATCH_CURSOR_EXPIRED') { cursor = ''; $('connection').textContent = 'Refreshing expired cursor…'; }
    else { showError({ message: error.reason || error.code }); $('connection').textContent = 'Reconnecting…'; }
    setTimeout(() => openWatch(version), 1500);
  });
}
async function selectConversation(id) {
  closeWatch(); generation++; providerQuery++; paused = false; cursor = ''; snapshot = null; activities = []; chain = Promise.resolve();
  scope.conversationId = id || ''; store('selection', scope);
  plans.select(); tasks.select(); feedback.select();
  currentSession = undefined; $('session-state').textContent = 'No session.';
  $('provider-call-status').textContent = 'No conversation selected.'; $('provider-calls').replaceChildren(); $('turn-controls').replaceChildren();
  $('messages').replaceChildren(); $('history').replaceChildren(); $('activity').replaceChildren(); historyBefore = '0'; historySnapshotId = ''; historyHasMore = true; historyIds = new Set();
  $('title').textContent = id ? 'Opening conversation…' : 'A place to work things through.';
  $('turn-state').textContent = 'No conversation selected.'; $('older').hidden = true; $('connection').textContent = id ? 'Connecting…' : 'Choose a conversation'; $('toggle-watch').textContent = 'Disconnect';
  for (const button of $('conversations').children) button.setAttribute('aria-current', String(button.dataset.id === id));
  showEmpty(); controls(); openWatch(generation); if (id) await loadSession();
}
$('agent-configuration').onsubmit = event => {
  event.preventDefault();
  const proposed = Object.fromEntries(new FormData(event.target));
  submit('UpdateAgentConfiguration', { expectedRevisionId: configuration.revisionId, configuration: { ...proposed, workspaceId: scope.workspaceId } });
};
$('refresh-session').onclick = () => loadSession().catch(showError);
$('refresh-provider-calls').onclick = () => loadProviderCalls().catch(showError);
$('replace-session').onclick = () => submit('ReplaceConversationSession', { ...scopeBody(), expectedGeneration: currentSession.generation });
$('create-workspace').onsubmit = event => { event.preventDefault(); submit('CreateWorkspace', Object.fromEntries(new FormData(event.target))); };
$('create-project').onsubmit = event => { event.preventDefault(); submit('CreateProject', { ...Object.fromEntries(new FormData(event.target)), workspaceId: scope.workspaceId }); };
$('create-conversation').onsubmit = event => { event.preventDefault(); submit('CreateConversation', { ...Object.fromEntries(new FormData(event.target)), workspaceId: scope.workspaceId, projectId: scope.projectId }); };
$('send').onsubmit = event => { event.preventDefault(); submit('SendMessage', { ...scopeBody(), content: $('content').value }); };
$('retry').onclick = retry;
$('refresh').onclick = () => loadDiscovery().catch(showError);
$('workspace').onchange = async () => {
  try {
    if ($('workspace').value === '__more__') { $('workspace').lastElementChild.remove(); await workspacePage(workspaceAfter); $('workspace').value = scope.workspaceId || ''; return; }
    scope = { workspaceId: $('workspace').value }; await selectConversation(''); await loadDiscovery();
  } catch (error) { showError(error); }
};
$('project').onchange = async () => {
  try {
    if ($('project').value === '__more__') { $('project').lastElementChild.remove(); await projectPage(projectAfter); $('project').value = scope.projectId || ''; return; }
    scope = { workspaceId: scope.workspaceId, projectId: $('project').value }; await selectConversation(''); await loadDiscovery();
  } catch (error) { showError(error); }
};
$('more-conversations').onclick = () => conversationPage(conversationAfter).catch(showError);
$('toggle-watch').onclick = () => { paused = !paused; if (paused) { closeWatch(); $('connection').textContent = 'Disconnected · work continues'; } else openWatch(generation); $('toggle-watch').textContent = paused ? 'Reconnect' : 'Disconnect'; };
$('older').onclick = async () => {
  const version = generation;
  try {
    historySnapshotId ||= snapshot.historySnapshotId;
    const result = await api('ListConversationHistory', { ...scopeBody(), snapshotId: historySnapshotId, beforeSequence: historyBefore, limit: 50 });
    if (version !== generation) return;
    historyBefore = result.nextBeforeSequence || '0';
    const fragment = document.createDocumentFragment();
    for (const record of [...(result.records || [])].reverse()) {
      if (historyIds.has(record.id) || (snapshot.recentMessages || []).some(message => message.id === record.id)) continue;
      historyIds.add(record.id); const container = document.createElement('div'); addMessage(container, record.roleOrState?.includes('USER') ? 'You' : 'History', record.contentOrOutcome); fragment.append(...container.children);
    }
    $('history').prepend(fragment);
    historyHasMore = !!result.hasMore; $('older').hidden = !historyHasMore;
  } catch (error) { if (version === generation) { if (error.reason === 'ERROR_REASON_HISTORY_SNAPSHOT_EXPIRED') { historyBefore = '0'; historySnapshotId = ''; historyIds.clear(); $('history').replaceChildren(); } showError(error); } }
};
controls();
loadDiscovery().then(() => selectConversation(scope.conversationId)).catch(showError);
