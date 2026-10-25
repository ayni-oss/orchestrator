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
let sessions = [];
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
const send = async (tab, content) => {
  await tab.getByRole('textbox', { name: 'Message', exact: true }).fill(content);
  await tab.getByRole('button', { name: 'Send message', exact: false }).click();
  await tab.locator('.transcript .message-body p').filter({ hasText: content }).first().waitFor();
  await completed(tab);
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
  const content = 'Explain this change <img src=x onerror="window.injected=true">';
  await send(page, content);
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
    const response = await fetch('/api/ListExecutionTimeline', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify({ ...scope, projectId: 'foreign-project' }) });
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
  violations.push(...await page.evaluate(() => window.cspViolations));
  assert.deepEqual(errors, []); assert.deepEqual(violations, []);
  const report = { passed: true, mode: fixture ? 'browser-fixture' : 'deployed', sessions, checks: ['create', 'chat', 'reload', 'two-tabs', 'reconnect', 'exact-command-retry', 'capacity-rejection', 'isolation', 'history', 'retained-execution-timeline', 'timeline-refresh', 'timeline-scope-isolation', 'content-escaping', 'scoped-console-handoff', 'themes', 'mobile', 'content-security-policy'] };
  report.checks.push(fixture ? 'provider-usage-display-fixture' : 'distinct-retained-session-identities');
  if (evidence) await writeFile(`${evidence}/outcome.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); await fixture?.close(); }
