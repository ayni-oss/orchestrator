// Run against an already deployed local stack and `dev/ctl console`.
// Each run creates its own workspace and preserves accepted application data.
import { chromium } from 'playwright';
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
const completed = () => page.waitForFunction(() => document.querySelector('#turn-state').textContent === 'All accepted turns have finished.');
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
  await page.locator('#create-project').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#create-project input').fill('Scripted workflow');
  await page.locator('#create-project button').click();
  await page.waitForFunction(() => !!document.querySelector('#project').value);
  await page.locator('#create-conversation').evaluate(form => { form.closest('details').open = true; });
  await page.locator('#create-conversation input').fill('Durable conversation');
  await page.locator('#create-conversation button').click();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  const content = 'A browser message <img src=x onerror="window.injected=true">';
  await page.locator('#content').fill(content);
  await page.locator('#send button').click();
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
  console.log(JSON.stringify({ outcome: 'passed', workspace: run, checks: ['validation-correction', 'create/select', 'scripted-response', 'text-only-content', 'refresh', 'two-tabs', 'disconnect-replay', 'ambiguous-command-reload-retry', 'conversation-isolation', 'older-history', 'typed-cursor-expiry-fixture', 'slow-browser-replay-fixture', 'mobile-layout'] }));
} finally { await browser.close(); }
