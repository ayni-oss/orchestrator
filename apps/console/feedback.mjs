const $ = id => document.getElementById(id);
const scopeKey = scope => JSON.stringify([scope.workspaceId, scope.projectId, scope.conversationId || '']);

// A late read cannot replace feedback from another selection or a newer refresh.
export function createFeedbackList({ getScope, api, projectWide = false, changed = () => {}, failed = () => {} }) {
  let query = 0;
  let state = { items: [], next: '', loading: false, error: false };
  const selected = () => {
    const scope = getScope();
    return { workspaceId: scope.workspaceId, projectId: scope.projectId, ...(!projectWide && { conversationId: scope.conversationId }) };
  };
  function reset() {
    query++;
    state = { items: [], next: '', loading: false, error: false };
    changed(state);
  }
  async function load(more = false) {
    const scope = selected();
    if (!scope.workspaceId || !scope.projectId || (!projectWide && !scope.conversationId)) return;
    if (more && (state.loading || !state.next)) return;
    const version = ++query;
    const current = () => version === query && scopeKey(scope) === scopeKey(selected());
    state = { ...state, loading: true, error: false }; changed(state);
    try {
      const page = await api('ListTaskFeedback', { ...scope, afterId: more ? state.next : '', limit: 20 });
      if (!current()) return;
      const items = page.feedback || [];
      if (items.some(item => item.workspaceId !== scope.workspaceId || item.projectId !== scope.projectId
        || (!projectWide && item.conversationId !== scope.conversationId))) {
        throw new Error('Task feedback did not match the selected scope. Refresh to try again.');
      }
      const unique = new Map((more ? state.items : []).map(item => [item.id, item]));
      for (const item of items) unique.set(item.id, item);
      state = { items: [...unique.values()], next: page.nextAfterId || '', loading: false, error: false };
      changed(state);
    } catch (error) {
      if (!current()) return;
      state = { items: [], next: '', loading: false, error: true }; changed(state); failed(error);
    }
  }
  return { reset, load, state: () => state };
}

export function mountFeedback({ getScope, api, showError, openTask }) {
  function render(prefix, projectWide, state) {
    const scope = getScope();
    const available = !!scope.projectId && (projectWide || !!scope.conversationId);
    const list = $(`${prefix}-list`); list.replaceChildren();
    $(`${prefix}-refresh`).disabled = !available || state.loading;
    $(`${prefix}-more`).hidden = !state.next;
    $(`${prefix}-more`).disabled = state.loading;
    $(`${prefix}-status`).textContent = !available ? `Choose a ${projectWide ? 'project' : 'conversation'}.`
      : state.error ? 'Feedback could not be loaded. Refresh to try again.'
      : state.loading ? 'Loading task feedback…'
      : state.items.length ? `${state.items.length} retained item${state.items.length === 1 ? '' : 's'} shown${state.next ? ' · more available' : ''}.`
      : state.next ? 'No open items on this page. Load more to inspect remaining feedback.'
      : `No task feedback needs attention in this ${projectWide ? 'project' : 'conversation'}.`;
    for (const item of state.items) {
      const article = document.createElement('article'); article.className = 'task-feedback'; article.dataset.feedbackId = item.id;
      const heading = document.createElement('h3'); heading.textContent = `${item.category || 'Task'} · ${item.state === 'resolved' ? 'recovery authorized' : 'needs attention'}`;
      const summary = document.createElement('p'); summary.textContent = item.summary || 'Inspect the task for its retained outcome.';
      const origin = document.createElement('small'); origin.textContent = `Task ${item.taskId} · attempt ${item.attemptId}${projectWide ? ` · conversation ${item.conversationId}` : ''}`;
      const button = document.createElement('button'); button.type = 'button'; button.textContent = projectWide ? 'Open originating conversation' : 'Inspect task';
      const selected = { workspaceId: item.workspaceId, projectId: item.projectId, conversationId: item.conversationId };
      button.onclick = () => {
        const current = getScope();
        if (current.workspaceId === selected.workspaceId && current.projectId === selected.projectId
          && (projectWide || current.conversationId === selected.conversationId)) {
          Promise.resolve(openTask(selected, item.taskId, projectWide, item.attemptId)).catch(showError);
        }
      };
      article.append(heading, summary, origin, button);
      if (item.recoveryAttemptId) {
        const outcome = document.createElement('p'); outcome.dataset.recoveryOutcome = item.recoveryAttemptId;
        outcome.textContent = `Continued attempt ${item.recoveryAttemptId} · coding: ${item.recoveryState || 'unknown'} · quality: ${item.recoveryQualityState || 'not available'}. Earlier evidence and possible charges remain retained.`;
        const continued = document.createElement('button'); continued.type = 'button'; continued.textContent = 'Inspect continued outcome';
        continued.onclick = () => {
          const current = getScope();
          if (current.workspaceId === selected.workspaceId && current.projectId === selected.projectId && (projectWide || current.conversationId === selected.conversationId)) Promise.resolve(openTask(selected, item.taskId, projectWide, item.recoveryAttemptId)).catch(showError);
        };
        article.append(outcome, continued);
      }
      list.append(article);
    }
  }
  const project = createFeedbackList({ getScope, api, projectWide: true, changed: state => render('project-attention', true, { ...state, items: state.items.filter(item => item.state !== 'resolved') }), failed: showError });
  const conversation = createFeedbackList({ getScope, api, changed: state => render('conversation-feedback', false, state), failed: showError });
  for (const [prefix, list] of [['project-attention', project], ['conversation-feedback', conversation]]) {
    $(`${prefix}-refresh`).onclick = () => list.load();
    $(`${prefix}-more`).onclick = () => list.load(true);
  }
  function select() {
    project.reset(); conversation.reset();
    project.load(); conversation.load();
  }
  return { select, refresh: () => Promise.all([project.load(), conversation.load()]) };
}
