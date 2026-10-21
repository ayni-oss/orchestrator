// Run against an already deployed local stack and `dev/ctl console`.
// Each run creates its own workspace and preserves accepted application data.
import { chromium } from 'playwright';
import { exercisePlans } from './plan-acceptance.mjs';
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
const endpoint = process.env.CONSOLE_URL || 'http://127.0.0.1:3000';
const url = new URL(endpoint);
assert.equal(url.hostname, '127.0.0.1', 'browser acceptance requires the local console');
assert.equal(url.protocol, 'http:');
const evidence = process.env.CONSOLE_EVIDENCE_DIRECTORY;
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
const run = `Console acceptance ${Date.now()}`;
const completed = () => page.waitForFunction(() => document.querySelector('#turn-state').textContent === 'All accepted turns have finished.', null, { timeout: 120000 });
const gateway = async (tab, method, body) => {
  const result = await tab.evaluate(async ({ method, body }) => {
    const response = await fetch(`/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body), signal: AbortSignal.timeout(15000) });
    return { status: response.status, body: await response.json() };
  }, { method, body });
  assert.equal(result.status, 200, `${method}: ${JSON.stringify(result.body)}`);
  return result.body;
};
const selection = tab => tab.evaluate(() => JSON.parse(sessionStorage.getItem('selection')));
const saveInstructions = async (instructions) => {
  const previous = await page.locator('#configuration-status').textContent();
  await page.locator('#agent-configuration').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#agent-configuration [name="instructions"]').fill(instructions);
  const saved = page.waitForResponse(response => new URL(response.url()).pathname === '/api/UpdateAgentConfiguration');
  await page.locator('#agent-configuration button').click();
  const response = await saved;
  assert.equal(response.status(), 200);
  const result = await response.json();
  await page.waitForFunction(previous => document.querySelector('#configuration-status').textContent !== previous && document.querySelector('#configuration-status').textContent.includes('interface v1'), previous);
  return result.configuration;
};
try {
  await page.goto(endpoint);
  await page.locator('#workspace').waitFor({ state: 'visible' });
  await page.locator('#create-workspace').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#create-workspace input').fill(' Invalid padded name ');
  await page.locator('#create-workspace button').click();
  await page.locator('#error').waitFor({ state: 'visible' });
  assert.equal(await page.locator('#pending').isVisible(), false);
  await page.locator('#create-workspace input').fill(run);
  await page.locator('#create-workspace button').click();
  await page.waitForFunction(() => !!document.querySelector('#workspace').value);
  await page.locator('#agent-configuration button').waitFor({ state: 'attached' });
  const firstInstructions = `Instructions for ${run} <script>not executable</script>`;
  const firstConfiguration = await saveInstructions(firstInstructions);
  assert.equal(firstConfiguration.provider, 'scripted');
  assert.equal(firstConfiguration.model, 'scripted');
  await page.locator('#create-project').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#create-project input').fill('Scripted workflow');
  await page.locator('#create-project button').click();
  await page.waitForFunction(() => !!document.querySelector('#project').value);
  await page.locator('#create-conversation').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#create-conversation input').fill('Durable conversation');
  await page.locator('#create-conversation button').click();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  const content = 'A browser message <img src=x onerror="window.injected=true">';
  // A canonical capacity rejection is definitive non-admission. The browser
  // releases its pending key and permits the same content as a new command.
  // Real capacity rollback/reservation behavior belongs to the PostgreSQL gate.
  await page.route('**/api/SendMessage', route => route.fulfill({ status: 429, contentType: 'application/json', body: JSON.stringify({ code: 'ResourceExhausted', reason: 'ERROR_REASON_BOUND_EXCEEDED', requestId: 'capacity-browser-fixture' }) }), { times: 1 });
  await page.locator('#content').fill(content);
  const rejectedAdmission = page.waitForResponse(response => new URL(response.url()).pathname === '/api/SendMessage');
  await page.locator('#send button').click();
  const rejectedResponse = await rejectedAdmission;
  assert.equal(rejectedResponse.status(), 429);
  const rejectedKey = rejectedResponse.request().postDataJSON().idempotencyKey;
  await page.locator('#error').waitFor({ state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('#send button').disabled);
  assert.match(await page.locator('#error').textContent(), /ERROR_REASON_BOUND_EXCEEDED/);
  assert.equal(await page.locator('#pending').isVisible(), false);
  assert.equal(await page.evaluate(() => sessionStorage.getItem('pending')), null);
  assert.equal(await page.locator('#content').inputValue(), content);
  const rejectedSnapshot = (await gateway(page, 'GetConversationSnapshot', await selection(page))).snapshot;
  assert.equal((rejectedSnapshot.recentMessages || []).length, 0);
  assert.equal((rejectedSnapshot.nonterminalTurns || []).length, 0);
  await page.locator('#content').fill(content);
  const firstAdmission = page.waitForResponse(response => new URL(response.url()).pathname === '/api/SendMessage');
  await page.locator('#send button').click();
  const firstResponse = await firstAdmission;
  assert.notEqual(firstResponse.request().postDataJSON().idempotencyKey, rejectedKey);
  const firstReceipt = (await firstResponse.json()).receipt;
  const firstScope = await selection(page);
  const firstInputs = await gateway(page, 'GetTurnInputs', { ...firstScope, turnId: firstReceipt.turnId });
  assert.deepEqual(firstInputs.configuration, firstConfiguration);
  await page.getByText(content, { exact: true }).waitFor();
  await completed();
  assert.equal(await page.locator('#messages .user').count(), 1);
  assert.equal(await page.locator('#messages img').count(), 0);
  await page.reload();
  await page.getByText(content, { exact: true }).waitFor();
  await completed();
  // A second tab discovers the same hierarchy and converges on the same messages.
  const second = await context.newPage();
  await second.goto(endpoint);
  await second.locator('#workspace').selectOption({ label: run });
  await second.locator('#project').selectOption({ label: 'Scripted workflow' });
  await second.getByRole('button', { name: 'Durable conversation', exact: true }).click();
  await second.getByText(content, { exact: true }).waitFor();
  // Both conversations share workspace configuration, but admitted turns keep
  // their immutable input revision after configuration edits or conflicting saves.
  await second.locator('#agent-configuration').evaluate(form => { form.closest('details').open = true; });
  assert.equal(await second.locator('#agent-configuration [name="instructions"]').inputValue(), firstInstructions);
  const nextInstructions = `New instructions for ${run}`;
  const nextConfiguration = await saveInstructions(nextInstructions);
  assert.notEqual(nextConfiguration.revisionId, firstConfiguration.revisionId);
  const retainedInputs = await gateway(page, 'GetTurnInputs', { ...firstScope, turnId: firstReceipt.turnId });
  assert.deepEqual(retainedInputs.configuration, firstConfiguration);
  await second.locator('#agent-configuration [name="instructions"]').fill('Stale configuration must not overwrite accepted instructions');
  const conflict = second.waitForResponse(response => new URL(response.url()).pathname === '/api/UpdateAgentConfiguration');
  await second.locator('#agent-configuration button').click();
  assert.notEqual((await conflict).status(), 200);
  await second.locator('#error').waitFor({ state: 'visible' });
  assert.equal(await second.locator('#pending').isVisible(), false);
  assert.deepEqual((await gateway(page, 'GetAgentConfiguration', { workspaceId: firstScope.workspaceId })).configuration, nextConfiguration);
  await second.locator('#refresh').click();
  await second.waitForFunction(expected => document.querySelector('#agent-configuration [name="instructions"]').value === expected, nextInstructions);
  await page.locator('#toggle-watch').click();
  await second.locator('#content').fill('Written from the second tab');
  await second.locator('#send button').click();
  await second.getByText('Written from the second tab', { exact: true }).waitFor();
  assert.equal(await page.getByText('Written from the second tab', { exact: true }).count(), 0);
  await page.locator('#toggle-watch').click();
  await page.getByText('Written from the second tab', { exact: true }).waitFor();
  await completed();
  // Lose the HTTP response after the backend has accepted the command, reload,
  // and retry the original retained browser command. Only one turn is admitted.
  await page.route('**/api/SendMessage', async route => { await route.fetch(); await route.abort('failed'); }, { times: 1 });
  await page.locator('#content').fill('Accepted before the response was lost');
  await page.locator('#send button').click();
  await page.locator('#error').waitFor({ state: 'visible' });
  await page.reload();
  await page.locator('#pending').waitFor({ state: 'visible' });
  await page.locator('#retry').click();
  await page.locator('#pending').waitFor({ state: 'hidden' });
  await page.getByText('Accepted before the response was lost', { exact: true }).waitFor();
  await completed();
  assert.equal(await page.getByText('Accepted before the response was lost', { exact: true }).count(), 1);
  // Create a separate conversation; no transcript from the first may leak.
  await page.locator('#create-conversation').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#create-conversation input').fill('Isolated conversation');
  await page.locator('#create-conversation button').click();
  await page.waitForFunction(() => document.querySelector('#title').textContent === 'Isolated conversation');
  assert.equal(await page.getByText(content, { exact: true }).count(), 0);
  const isolatedScope = await selection(page);
  const isolatedAdmission = page.waitForResponse(response => new URL(response.url()).pathname === '/api/SendMessage');
  await page.locator('#content').fill('Only the isolated conversation can see this');
  await page.locator('#send button').click();
  const isolatedReceipt = (await (await isolatedAdmission).json()).receipt;
  await page.getByText('Only the isolated conversation can see this', { exact: true }).waitFor();
  await completed();
  const isolatedInputs = await gateway(page, 'GetTurnInputs', { ...isolatedScope, turnId: isolatedReceipt.turnId });
  assert.deepEqual(isolatedInputs.configuration, nextConfiguration);
  const firstSessions = (await gateway(page, 'ListConversationSessions', firstScope)).sessions;
  const isolatedSessions = (await gateway(page, 'ListConversationSessions', isolatedScope)).sessions;
  assert.ok(firstSessions.length && isolatedSessions.length);
  assert.notEqual(firstSessions[0].id, isolatedSessions[0].id);
  assert.ok(firstSessions.every(session => session.conversationId === firstScope.conversationId));
  assert.ok(isolatedSessions.every(session => session.conversationId === isolatedScope.conversationId));
  await page.getByRole('button', { name: 'Durable conversation', exact: true }).click();
  await page.getByText(content, { exact: true }).waitFor();
  // More than sixteen messages exercise the stable older-history boundary.
  for (let index = 0; index < 7; index++) {
    await page.locator('#content').fill(`History sample ${index}`);
    await page.locator('#send button').click();
    await page.getByText(`History sample ${index}`, { exact: true }).waitFor();
    await completed();
  }
  await page.locator('#older').click();
  await page.locator('#history .message').first().waitFor();
  await page.getByText(content, { exact: true }).waitFor();
  // An accepted idle replacement survives a lost HTTP response. Retrying the
  // retained command fences exactly that generation; the next message wakes a
  // new generation while the old conversation history remains available.
  await page.locator('#session-state').evaluate(element => { element.closest('details').open = true; });
  const replacementDeadline = Date.now() + 60000;
  while (true) {
    const refreshed = page.waitForResponse(response => new URL(response.url()).pathname === '/api/ListConversationSessions');
    await page.locator('#refresh-session').click();
    const result = await (await refreshed).json();
    if (result.sessions?.[0]?.canReplace) break;
    assert.ok(Date.now() < replacementDeadline, 'accepted work must settle before replacement becomes available');
    await page.waitForTimeout(100);
  }
  await page.waitForFunction(() => !document.querySelector('#replace-session').disabled);
  const beforeReplacement = (await gateway(page, 'ListConversationSessions', firstScope)).sessions[0];
  await page.route('**/api/ReplaceConversationSession', async route => { const accepted = await route.fetch(); assert.equal(accepted.status(), 200); await route.abort('failed'); }, { times: 1 });
  await page.locator('#replace-session').click();
  await page.locator('#error').waitFor({ state: 'visible' });
  await page.locator('#pending').waitFor({ state: 'visible' });
  await page.reload();
  await page.locator('#pending').waitFor({ state: 'visible' });
  await page.locator('#retry').click();
  await page.locator('#pending').waitFor({ state: 'hidden' });
  const replaced = (await gateway(page, 'ListConversationSessions', firstScope)).sessions;
  assert.equal(replaced[0].generation, beforeReplacement.generation);
  assert.equal(replaced[0].state, 'replaced');
  await page.locator('#content').fill('Wake the replacement session');
  await page.locator('#send button').click();
  await page.getByText('Wake the replacement session', { exact: true }).waitFor();
  await completed();
  const awakened = (await gateway(page, 'ListConversationSessions', firstScope)).sessions;
  assert.equal(BigInt(awakened[0].generation), BigInt(beforeReplacement.generation) + 1n);
  assert.equal(awakened[0].id, beforeReplacement.id);
  assert.equal(awakened[1].state, 'replaced');
  // Inject the canonical typed-expiry frame at the browser boundary; the real
  // retention/expiry behavior is exercised by the cumulative PostgreSQL gate.
  await page.route('**/watch?**', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: 'event: fault\ndata: {"code":"FailedPrecondition","reason":"ERROR_REASON_WATCH_CURSOR_EXPIRED"}\n\n' }), { times: 1 });
  await page.locator('#toggle-watch').click();
  const resetRequest = page.waitForRequest(request => new URL(request.url()).pathname === '/watch' && !new URL(request.url()).searchParams.has('afterCursor'));
  await page.locator('#toggle-watch').click();
  await resetRequest;
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  // A replay burst must release the slow browser stream and catch up from its
  // processed cursor, keeping the local promise queue bounded.
  await exercisePlans({ page, context, endpoint, gateway, scope: firstScope });
  const retainedCursor = await page.locator('#cursor').textContent();
  const burst = Array.from({ length: 80 }, () => `event: frame\ndata: ${JSON.stringify({ update: { cursor: retainedCursor, conversationRevision: '1' } })}\n\n`).join('');
  await page.route('**/watch?**', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: burst }), { times: 1 });
  await page.locator('#toggle-watch').click();
  await page.locator('#toggle-watch').click();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Catching up…');
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await page.setViewportSize({ width: 1440, height: 1000 });
  if (evidence) { await mkdir(evidence, { recursive: true }); await page.screenshot({ path: `${evidence}/console.png`, fullPage: true }); }
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ outcome: 'passed', workspace: run, checks: ['validation-correction', 'typed-capacity-rejection-resubmit-fixture', 'create/select', 'scripted-response', 'text-only-content', 'refresh', 'two-tabs', 'disconnect-replay', 'ambiguous-command-reload-retry', 'conversation-isolation', 'configuration-editor', 'immutable-turn-inputs', 'stale-configuration-conflict', 'session-isolation', 'session-replacement-retry', 'session-generation-wakeup', 'older-history', 'typed-cursor-expiry-fixture', 'slow-browser-replay-fixture', 'mobile-layout'] }));
} finally { await browser.close(); }
