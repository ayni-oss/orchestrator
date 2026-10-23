// Presentation fixtures cover browser races and text rendering. PostgreSQL and
// deployed acceptance own feedback generation, deduplication and persistence.
import assert from 'node:assert/strict';

export async function exerciseFeedback({ page, gateway, scope }) {
  const canonical = await gateway(page, 'ListTaskFeedback', { ...scope, limit: 20 });
  assert.equal((canonical.feedback || []).length, 0);
  const task = (await gateway(page, 'ListTasks', { ...scope, limit: 1 })).tasks[0];
  const other = (await gateway(page, 'CreateConversation', { workspaceId: scope.workspaceId, projectId: scope.projectId, title: 'Feedback isolation', idempotencyKey: crypto.randomUUID() })).conversation;
  const summary = '<img src=x onerror="window.feedbackInjected=true"> Quality needs review.';
  const first = { ...scope, id: crypto.randomUUID(), taskId: task.id, attemptId: crypto.randomUUID(), category: 'execution', state: 'open', safeReason: 'task_interrupted', summary };
  const second = { ...first, id: crypto.randomUUID(), category: 'quality', summary: 'Inspect the retained quality evidence.' };
  let delayed;
  let delayNext = false;
  await page.route('**/api/ListTaskFeedback', async route => {
    const body = route.request().postDataJSON();
    assert.equal(body.limit, 20);
    assert.equal(body.workspaceId, scope.workspaceId); assert.equal(body.projectId, scope.projectId);
    if (body.conversationId === scope.conversationId && delayNext) {
      delayNext = false;
      await new Promise(resolve => { delayed = resolve; });
      return route.fulfill({ json: { feedback: [{ ...first, summary: 'STALE FEEDBACK MUST NOT RENDER' }] } });
    }
    const result = body.conversationId && body.conversationId !== scope.conversationId ? {}
      : body.afterId ? { feedback: [second] } : { feedback: [first], nextAfterId: first.id };
    return route.fulfill({ json: result });
  });
  await page.locator('#refresh').click();
  await page.locator(`#conversations [data-id="${other.id}"]`).waitFor();
  await page.locator('#conversation-feedback-refresh').click();
  await page.locator('#conversation-feedback-list [data-feedback-id]').waitFor();
  assert.match(await page.locator('#conversation-feedback-list').innerText(), /Quality needs review/);
  assert.equal(await page.locator('#conversation-feedback-list img').count(), 0);
  assert.equal(await page.evaluate(() => window.feedbackInjected), undefined);
  await page.locator('#conversation-feedback-more').click();
  await page.waitForFunction(() => document.querySelector('#conversation-feedback-list').children.length === 2);
  await page.locator('#project-attention-refresh').click();
  await page.locator('#project-attention-more').waitFor({ state: 'visible' });
  await page.locator('#project-attention-more').click();
  await page.waitForFunction(() => document.querySelector('#project-attention-list').children.length === 2);
  await page.reload();
  await page.locator('#conversation-feedback-list [data-feedback-id]').waitFor();
  // Selection and the initial watch snapshot each refresh feedback. Finish both
  // before deliberately delaying one read, so its refresh button stays usable.
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected'
    && document.querySelector('#cursor').textContent !== '—'
    && !document.querySelector('#conversation-feedback-refresh').disabled);
  assert.match(await page.locator('#conversation-feedback-list').innerText(), /needs attention/);
  delayNext = true;
  const late = page.waitForRequest(request => request.url().endsWith('/api/ListTaskFeedback') && request.postDataJSON().conversationId === scope.conversationId);
  await page.locator('#conversation-feedback-refresh').click(); await late;
  await page.locator(`#conversations [data-id="${other.id}"]`).click();
  await page.waitForFunction(() => document.querySelector('#conversation-feedback-status').textContent.includes('No task feedback'));
  delayed();
  await page.waitForTimeout(100);
  assert.equal(await page.locator('#conversation-feedback-list').innerText(), '');
  assert.equal(await page.getByText('STALE FEEDBACK MUST NOT RENDER', { exact: true }).count(), 0);
  // The project catalog points back to the immutable originating conversation.
  await page.locator('#project-attention-list button').first().click();
  await page.waitForFunction(id => JSON.parse(sessionStorage.getItem('selection')).conversationId === id, scope.conversationId);
  await page.waitForFunction(id => document.querySelector('#task-status').dataset.taskId === id, task.id);
  await page.locator('#conversation-feedback-list button').first().click();
  assert.equal(await page.locator('#conversation-feedback-list [data-task-action]').count(), 0);
  await page.unroute('**/api/ListTaskFeedback');
  await page.locator('#conversation-feedback-refresh').click();
  await page.waitForFunction(() => document.querySelector('#conversation-feedback-list').children.length === 0);
  console.log(JSON.stringify({ taskFeedbackConsole: 'pass', cases: ['canonical-scoped-read', 'conversation-and-project-pagination', 'text-only-summary', 'reload', 'stale-conversation-response-fencing', 'origin-navigation', 'inspect-retained-task', 'no-recovery-authority'] }));
}
