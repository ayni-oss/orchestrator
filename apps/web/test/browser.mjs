import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { startFixture } from './fixture.mjs';
const fixture = process.env.WEB_URL ? null : await startFixture();
const endpoint = process.env.WEB_URL || fixture.url;
const url = new URL(endpoint);
assert.equal(url.hostname, '127.0.0.1'); assert.equal(url.protocol, 'http:');
const evidence = process.env.WEB_EVIDENCE_DIRECTORY;
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1600, height: 1080 } });
const page = await context.newPage();
const errors = [], violations = [];
page.on('pageerror', error => errors.push(error.message));
await context.addInitScript(() => { window.cspViolations = []; document.addEventListener('securitypolicyviolation', event => window.cspViolations.push(event.violatedDirective)); });
const run = `Workspace ${Date.now()}`;
let sessions = [], taskEvidence;
const requestedTask = process.env.WEB_TASK_SCOPE ? JSON.parse(process.env.WEB_TASK_SCOPE) : null;
if (requestedTask) for (const key of ['workspaceId', 'projectId', 'conversationId', ...['taskId', 'attemptId'].filter(key => requestedTask[key])]) assert.match(requestedTask[key] || '', /^[a-zA-Z0-9_-]{1,128}$/);
const currentScope = tab => tab.evaluate(() => JSON.parse(sessionStorage.getItem('ayni.workspace.selection')));
const api = async (tab, method, body) => tab.evaluate(async ({ method, body }) => {
  const response = await fetch(`/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body) });
  if (!response.ok) throw new Error(`Canonical read failed: ${method}`);
  return response.json();
}, { method, body });
const create = async (kind, name, tab = page) => {
  await tab.getByRole('button', { name: `Create ${kind}`, exact: true }).first().click();
  const dialog = tab.getByRole('dialog'); await dialog.getByLabel(kind === 'conversation' ? 'Title' : 'Name', { exact: true }).fill(name);
  if (kind === 'project') await dialog.getByLabel('Project goal').fill('Review a durable project conversation from the product workspace.');
  await dialog.getByRole('button', { name: `Create ${kind}`, exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
};
const completed = tab => tab.getByText('All accepted turns have finished', { exact: true }).waitFor({ timeout: 120000 });
const send = async (tab, content, observe = false) => {
  await tab.getByRole('textbox', { name: 'Message', exact: true }).fill(content);
  await tab.getByRole('button', { name: 'Send message', exact: false }).click();
  await tab.locator('.transcript .message-body p').filter({ hasText: content }).first().waitFor();
  if (observe) {
    await tab.getByRole('article', { name: 'Provisional assistant output' }).waitFor();
    assert.equal(await tab.locator('.transcript .message:not(.from-user):not(.provisional-message)').count(), 0);
    assert.equal(await tab.locator('.provisional-message .badge').textContent(), 'PROVISIONAL');
  }
  await completed(tab);
  await tab.getByRole('article', { name: 'Provisional assistant output' }).waitFor({ state: 'hidden' });
};
try {
  await page.goto(endpoint);
  await create('workspace', run);
  await create('project', 'Observable delivery');
  await page.getByRole('button', { name: 'New conversation' }).click();
  await page.getByRole('dialog').getByLabel('Title', { exact: true }).fill('Product discussion');
  await page.getByRole('dialog').getByRole('button', { name: 'Create conversation' }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByText('Live', { exact: true }).waitFor();
  const content = 'Observe streaming while explaining this change <img src=x onerror="window.injected=true">';
  await send(page, content, true);
  assert.equal(await page.locator('.transcript img').count(), 0);
  assert.equal(await page.evaluate(() => window.injected), undefined);
  await page.reload(); await completed(page);
  assert.equal(await page.locator('.transcript .from-user').count(), 1);
  const firstScope = await currentScope(page);
  // Reload reconstructs durable timeline entries, independently of this tab's watch.
  const timeline = page.getByRole('list', { name: 'Execution timeline', exact: true });
  await timeline.locator('li strong').first().waitFor();
  const retainedTimeline = await api(page, 'ListExecutionTimeline', { ...firstScope, limit: 1 });
  assert.equal(retainedTimeline.entries.length, 1);
  const newestActivity = retainedTimeline.entries[0].activity.id;
  await page.getByRole('button', { name: 'Refresh timeline', exact: true }).click();
  await timeline.getByText(newestActivity, { exact: true }).waitFor({ state: 'attached' });
  const foreignTimeline = await page.evaluate(async scope => {
    const response = await fetch('/api/ListExecutionTimeline', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify({ ...scope, projectId: scope.workspaceId }) });
    return response.status;
  }, firstScope);
  assert.equal(foreignTimeline, 404);
  const handoff = await page.getByRole('link', { name: 'Contributor console' }).getAttribute('href');
  assert.match(handoff, /workspaceId=.+projectId=.+conversationId=/);
  const second = await context.newPage();
  await second.goto(`${url.origin}/workspace/${handoff.slice(1)}`);
  await completed(second);
  if (!fixture) {
    const advanced = await context.newPage();
    await advanced.goto(`${url.origin}${handoff}`);
    await advanced.waitForFunction(() => document.querySelector('#connection')?.textContent === 'Connected');
    assert.equal(await advanced.locator('#title').textContent(), 'Product discussion');
    await advanced.close();
  }
  await page.getByRole('button', { name: 'Pause updates' }).click();
  await send(second, 'Written from another tab');
  assert.equal(await page.getByText('Written from another tab', { exact: true }).count(), 0);
  await page.getByRole('button', { name: 'Reconnect', exact: true }).click();
  await page.getByText('Written from another tab', { exact: true }).waitFor();
  await completed(page);
  await page.getByText('New activity is available.', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Refresh timeline', exact: true }).click();
  await page.getByText('New activity is available.', { exact: true }).waitFor({ state: 'hidden' });
  // Lose an accepted response, then reload and retry the exact stored command.
  await page.route('**/api/SendMessage', async route => { await route.fetch(); await route.abort('failed'); }, { times: 1 });
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Accepted before response loss');
  await page.getByRole('button', { name: 'Send message', exact: false }).click();
  await page.getByRole('region', { name: 'Pending command' }).waitFor();
  await page.reload();
  await page.getByRole('button', { name: 'Retry the same command' }).click();
  await page.getByRole('region', { name: 'Pending command' }).waitFor({ state: 'hidden' });
  await completed(page);
  assert.equal(await page.getByText('Accepted before response loss', { exact: true }).count(), 1);
  // A definitive capacity rejection permits a fresh command rather than holding a key.
  await page.route('**/api/SendMessage', route => route.fulfill({ status: 429, contentType: 'application/json', body: JSON.stringify({ code: 'ResourceExhausted', reason: 'ERROR_REASON_BOUND_EXCEEDED' }) }), { times: 1 });
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Try after capacity is available');
  await page.getByRole('button', { name: 'Send message', exact: false }).click();
  await page.getByRole('alert').filter({ hasText: 'ERROR_REASON_BOUND_EXCEEDED' }).waitFor();
  assert.equal(await page.getByRole('region', { name: 'Pending command' }).count(), 0);
  await send(page, 'Try after capacity is available');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Private unsent draft');
  await page.getByRole('button', { name: 'New conversation' }).click();
  await page.getByRole('dialog').getByLabel('Title', { exact: true }).fill('Separate context');
  await page.getByRole('dialog').getByRole('button', { name: 'Create conversation' }).click();
  await page.getByText('Ready for your first message', { exact: true }).waitFor();
  assert.equal(await page.getByRole('textbox', { name: 'Message', exact: true }).inputValue(), '');
  assert.equal(await page.locator('.transcript .message').count(), 0);
  await page.reload();
  await page.getByText('Ready for your first message', { exact: true }).waitFor();
  assert.equal(await page.locator('.conversation-heading h2').textContent(), 'Separate context');
  await send(page, 'Only in this conversation');
  if (!fixture) {
    const isolatedScope = await currentScope(page);
    const firstSessions = (await api(page, 'ListConversationSessions', firstScope)).sessions;
    const isolatedSessions = (await api(page, 'ListConversationSessions', isolatedScope)).sessions;
    assert.ok(firstSessions.length && isolatedSessions.length);
    assert.notEqual(firstSessions[0].id, isolatedSessions[0].id);
    assert.ok(firstSessions.every(session => session.conversationId === firstScope.conversationId));
    assert.ok(isolatedSessions.every(session => session.conversationId === isolatedScope.conversationId));
    sessions = [...firstSessions, ...isolatedSessions];
  }
  await page.getByRole('button', { name: 'Product discussion' }).click();
  await completed(page);
  assert.equal(await page.getByText('Only in this conversation', { exact: true }).count(), 0);
  for (let i = 0; i < 5; i++) await send(page, `History sample ${i}`);
  await page.getByRole('button', { name: 'Load earlier history' }).click();
  await page.getByText(content, { exact: true }).waitFor();
  assert.equal(await page.getByText(content, { exact: true }).count(), 1);
  assert.equal(await page.locator('.transcript .from-user').count(), 9);
  await page.getByRole('button', { name: 'Use dark theme' }).click();
  if (evidence) { await mkdir(evidence, { recursive: true }); await page.screenshot({ path: `${evidence}/workspace-dark.png`, fullPage: true }); }
  await page.getByRole('button', { name: 'Use light theme' }).click();
  if (evidence) await page.screenshot({ path: `${evidence}/workspace.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await page.getByRole('button', { name: 'Toggle navigation' }).click();
  await page.getByLabel('Workspace', { exact: true }).waitFor({ state: 'visible' });
  await page.getByRole('button', { name: 'Close navigation' }).click();
  if (evidence) await page.screenshot({ path: `${evidence}/workspace-mobile.png`, fullPage: true });
  if (fixture) {
    // Contract-shaped provider metadata verifies distinct call identities and optional usage.
    await page.route('**/api/ListProviderCalls', route => route.fulfill({ contentType: 'application/json', body: JSON.stringify({ calls: [
      { callId: 'call-one', turnId: 'turn-one', provider: 'openai', model: 'fixture-model', state: 'completed', inputTokens: '0', outputTokens: '123', totalTokens: '123' },
      { callId: 'call-two', turnId: 'turn-two', provider: 'anthropic', model: 'fixture-model', state: 'uncertain' },
    ] }) }));
    await page.reload(); await completed(page);
    await page.getByText('Provider calls & usage', { exact: true }).click();
    await page.locator('.provider-call').nth(1).waitFor();
    assert.equal(await page.locator('.provider-call').count(), 2);
    assert.equal(await page.locator('.provider-call').first().locator('dd').first().textContent(), '0');
    assert.equal(await page.locator('.provider-call').nth(1).locator('dd').first().textContent(), 'Unknown');
    await page.getByText('The provider outcome is unknown.', { exact: false }).waitFor();
  }
  if (fixture) {
    // Losing progress reads and reloading observation must not resubmit accepted work.
    const streamed = 'Observe streaming after a lost progress response';
    await page.route('**/api/ListExecutionProgress', route => route.abort('failed'), { times: 3 });
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill(streamed);
    await page.getByRole('button', { name: 'Send message', exact: false }).click();
    await page.getByText('Live output is temporarily unavailable.', { exact: false }).waitFor();
    await page.getByRole('article', { name: 'Provisional assistant output' }).waitFor();
    assert.equal(await page.locator('.provisional-message img').count(), 0);
    await page.reload();
    await page.getByRole('article', { name: 'Provisional assistant output' }).waitFor();
    await completed(page);
    await page.getByRole('article', { name: 'Provisional assistant output' }).waitFor({ state: 'hidden' });
    assert.equal(await page.getByText(streamed, { exact: true }).count(), 1);
    const selectedScope = await currentScope(page);
    const taskId = '11111111-1111-4111-8111-111111111111';
    const attemptId = '22222222-2222-4222-8222-222222222222';
    const at = delta => new Date(Date.now() - delta).toISOString();
    const timing = { queueAcceptedAt: at(3000), claimAcceptedAt: at(2500), codingResultAcceptedAt: at(2000), qualityResultAcceptedAt: at(1000), executionStoppedAt: at(800), observedAt: at(0) };
    const task = { ...selectedScope, id: taskId, proposedTaskKey: 'inspect', planId: 'fixture-plan', specification: { objective: 'Inspect separate delivery outcomes' } };
    const taskAttempt = { taskId, attemptId, state: 'completed', executionOutcome: 'completed', providerState: 'settled', generation: '1', patch: 'Retained patch <img src=x onerror="window.injected=true">', quality: { state: 'failed', safeReason: 'quality_failed', completedTargets: '1', expectedTargets: '2', exitCode: 1, artifact: 'Retained quality evidence' }, timing };
    for (const [method, result] of Object.entries({ ListTasks: { tasks: [task] }, GetTask: { task }, GetTaskExecution: { preparation: { taskId, attemptId, createdAt: at(4000) } }, GetTaskAttempt: { attempt: taskAttempt }, ListTaskRecovery: { records: [{ id: 'fixture-question', kind: 'question', question: { text: 'Which contract needs updating?' }, author: 'fixture', createdAt: at(500) }] } })) {
      await page.route(`**/api/${method}`, route => route.fulfill({ contentType: 'application/json', body: JSON.stringify(result) }));
    }
    await page.setViewportSize({ width: 1600, height: 1080 });
    await page.getByText('Task execution & delivery', { exact: true }).click();
    await page.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
    await page.locator(`[data-task-id="${taskId}"]`).click();
    const coding = page.getByRole('region', { name: 'Coding evidence' });
    const quality = page.getByRole('region', { name: 'Quality evidence' });
    const publication = page.getByRole('region', { name: 'Publication evidence' });
    await coding.waitFor(); await quality.waitFor(); await publication.waitFor();
    assert.ok(await coding.getByText('completed', { exact: true }).count());
    assert.equal(await quality.getByText('failed', { exact: true }).count(), 1);
    assert.equal(await publication.getByText('not authorized', { exact: true }).count(), 1);
    await page.getByText('Which contract needs updating?', { exact: true }).waitFor();
    await page.getByText('Accepted patch', { exact: true }).click();
    assert.equal(await page.locator('.task-detail img').count(), 0);
    assert.equal(await page.evaluate(() => window.injected), undefined);
    if (evidence) await page.screenshot({ path: `${evidence}/task-inspector-fixture.png`, fullPage: true });
  }
  if (requestedTask) {
    assert.equal(fixture, null, 'WEB_TASK_SCOPE requires a deployed backend');
    const taskTab = await context.newPage();
    const { workspaceId, projectId, conversationId, attemptId } = requestedTask;
    let { taskId } = requestedTask;
    await taskTab.goto(`${url.origin}/workspace/?${new URLSearchParams({ workspaceId, projectId, conversationId })}`);
    await taskTab.getByText('Live', { exact: true }).waitFor();
    await taskTab.getByText('Task execution & delivery', { exact: true }).click();
    await taskTab.waitForFunction(() => document.querySelector('.task-inspector')?.dataset.listBusy === 'false');
    if (!taskId) taskId = (await api(taskTab, 'ListTasks', { workspaceId, projectId, conversationId, limit: 1 })).tasks?.[0]?.id;
    assert.ok(taskId, 'task scope must contain an admitted task');
    const button = taskTab.locator(`[data-task-id="${taskId}"]`);
    for (let pages = 0; !(await button.count()) && pages < 100; pages++) {
      const more = taskTab.getByRole('button', { name: 'Load more tasks', exact: true });
      if (!await more.count()) break;
      await more.click();
      await taskTab.waitForFunction(() => document.querySelector('.task-inspector')?.dataset.listBusy === 'false');
    }
    await button.click();
    await taskTab.getByRole('region', { name: 'Selected task evidence' }).waitFor();
    const task = (await api(taskTab, 'GetTask', { workspaceId, projectId, conversationId, taskId })).task;
    const preparation = (await api(taskTab, 'GetTaskExecution', { workspaceId, projectId, conversationId, taskId })).preparation;
    const selectedAttempt = attemptId || preparation?.attemptId;
    assert.ok(selectedAttempt, 'task fixture must retain a prepared attempt');
    const attempt = (await api(taskTab, 'GetTaskAttempt', { workspaceId, projectId, conversationId, taskId, attemptId: selectedAttempt })).attempt;
    await taskTab.waitForFunction(id => document.querySelector('.task-detail')?.dataset.attemptId === id, preparation.attemptId);
    if (attemptId && attemptId !== preparation.attemptId) {
      let inspected = preparation.attemptId;
      for (let depth = 0; inspected !== attemptId && depth < 32; depth++) {
        const current = (await api(taskTab, 'GetTaskAttempt', { workspaceId, projectId, conversationId, taskId, attemptId: inspected })).attempt;
        assert.ok(current.parentAttemptId, 'requested attempt is not in the selected recovery chain');
        await taskTab.getByRole('button', { name: 'Inspect previous attempt', exact: true }).click();
        inspected = current.parentAttemptId;
        await taskTab.waitForFunction(id => document.querySelector('.task-detail')?.dataset.attemptId === id, inspected);
      }
      assert.equal(inspected, attemptId, 'requested attempt exceeds bounded recovery inspection');
    }
    const blockers = await taskTab.getByRole('region', { name: 'Preparation evidence' }).locator('dt').filter({ hasText: /^Blockers$/ }).evaluate(element => element.nextElementSibling.textContent);
    assert.equal(blockers, (preparation?.blockers || []).join('\n') || 'None recorded');
    const coding = taskTab.getByRole('region', { name: 'Coding evidence' });
    const quality = taskTab.getByRole('region', { name: 'Quality evidence' });
    const publication = taskTab.getByRole('region', { name: 'Publication evidence' });
    await coding.waitFor(); await quality.waitFor(); await publication.waitFor();
    assert.equal(task.workspaceId, workspaceId); assert.equal(task.projectId, projectId); assert.equal(task.conversationId, conversationId);
    assert.equal(attempt.taskId, taskId);
    assert.ok(attempt.timing?.queueAcceptedAt, 'queue acceptance timing must be retained');
    assert.ok(attempt.timing?.observedAt, 'task timing read must identify its observation');
    assert.ok(await coding.getByText(attempt.state.replaceAll('_', ' '), { exact: true }).count());
    if (attempt.quality) assert.ok(await quality.getByText(attempt.quality.state.replaceAll('_', ' '), { exact: true }).count());
    taskEvidence = { workspaceId, projectId, conversationId, taskId, attemptId: selectedAttempt, state: attempt.state, executionOutcome: attempt.executionOutcome, quality: attempt.quality?.state, publication: attempt.publication?.state, timing: attempt.timing };
    if (evidence) await taskTab.screenshot({ path: `${evidence}/task-inspector.png`, fullPage: true });
    await taskTab.close();
  }
  violations.push(...await page.evaluate(() => window.cspViolations));
  assert.deepEqual(errors, []); assert.deepEqual(violations, []);
  const report = { passed: true, mode: fixture ? 'browser-fixture' : 'deployed', sessions, checks: ['create', 'chat', 'reload', 'two-tabs', 'reconnect', 'exact-command-retry', 'capacity-rejection', 'isolation', 'history', 'retained-execution-timeline', 'provisional-output', 'terminal-output-reconciliation', 'timeline-refresh', 'timeline-scope-isolation', 'content-escaping', 'scoped-console-handoff', 'themes', 'mobile', 'content-security-policy'] };
  if (taskEvidence) { report.taskEvidence = taskEvidence; report.checks.push('deployed-task-attempt-inspector', 'task-accepted-timing'); }
  if (fixture) report.checks.push('progress-read-recovery-fixture', 'provisional-reload-fixture', 'task-evidence-display-fixture');
  report.checks.push(fixture ? 'provider-usage-display-fixture' : 'distinct-retained-session-identities');
  if (evidence) await writeFile(`${evidence}/outcome.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); await fixture?.close(); }
