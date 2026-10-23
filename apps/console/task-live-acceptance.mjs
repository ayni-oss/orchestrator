// Real canonical task/quality/publication authority through the deployed local
// gateway. The supplied fixture task must have passed managed quality and be in
// a workspace the publisher is not bound to, so queued cancellation is deterministic.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
const endpoint = process.env.CONSOLE_URL || 'http://127.0.0.1:3000';
assert.equal(new URL(endpoint).hostname, '127.0.0.1');
assert.equal(new URL(endpoint).protocol, 'http:');
const evidence = process.env.TASK_CONSOLE_EVIDENCE_DIRECTORY;
assert.ok(evidence?.startsWith('/'), 'absolute evidence directory required');
const coding = JSON.parse(await readFile(process.env.TASK_CONSOLE_SELECTION_FILE, 'utf8'));
const scope = { workspaceId: coding.scope.workspace, projectId: coding.scope.project, conversationId: coding.scope.conversation };
const browser = await chromium.launch();
try {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await context.addInitScript(scope => sessionStorage.setItem('selection', JSON.stringify(scope)), scope);
  const page = await context.newPage(); const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(endpoint);
  await page.locator('[data-inspect-task]').first().click();
  await page.getByRole('button', { name: 'Review draft publication', exact: true }).waitFor();
  await page.locator('#task-auto-refresh').uncheck();
  await page.locator('[data-task-evidence=patch] summary').click();
  assert.match(await page.locator('#task-detail').innerText(), /completed/);
  assert.match(await page.locator('#task-detail').innerText(), /passed/);
  assert.match(await page.getByRole('heading', { name: 'Preparation', exact: true }).locator('..').innerText(), /Preparation blockers\s*None/);
  await page.getByRole('button', { name: 'Review draft publication', exact: true }).click();
  await page.locator('#task-publication-consent').waitFor();
  assert.match(await page.locator('#task-publication-review').innerText(), /https:\/\/github.com\/fixture\/repo.git/);
  assert.equal(await page.getByRole('button', { name: 'Authorize draft publication', exact: true }).isDisabled(), true);
  await page.locator('#task-publication-consent').check();
  let accepted;
  await page.route('**/api/StartTaskPublication', async route => {
    const response = await route.fetch(); assert.equal(response.status(), 200);
    accepted = { request: route.request().postDataJSON(), response: await response.json() };
    await route.abort('failed');
  }, { times: 1 });
  await page.getByRole('button', { name: 'Authorize draft publication', exact: true }).click();
  await page.locator('#pending').waitFor({ state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('#retry').disabled);
  await page.reload(); await page.locator('#pending').waitFor({ state: 'visible' });
  const replay = page.waitForResponse(r => r.url().endsWith('/api/StartTaskPublication'));
  await page.locator('#retry').click();
  const response = await replay; assert.equal(response.status(), 200);
  assert.deepEqual(response.request().postDataJSON(), accepted.request);
  assert.deepEqual(await response.json(), accepted.response);
  await page.getByRole('button', { name: 'Cancel draft publication', exact: true }).waitFor();
  const cancel = page.waitForResponse(r => r.url().endsWith('/api/CancelTaskPublication'));
  await page.getByRole('button', { name: 'Cancel draft publication', exact: true }).click();
  const cancelled = await cancel; assert.equal(cancelled.status(), 200);
  await page.waitForFunction(() => document.querySelector('#task-status').textContent.includes('Publication was cancelled'));
  assert.equal(await page.getByRole('button', { name: 'Review draft publication', exact: true }).count(), 0);
  const retained = await page.evaluate(async body => {
    const response = await fetch('/api/GetTaskAttempt', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body) });
    if (!response.ok) throw new Error('canonical attempt read failed');
    return (await response.json()).attempt;
  }, { ...scope, taskId: accepted.request.taskId, attemptId: accepted.request.attemptId });
  assert.equal(retained.executionOutcome, 'completed'); assert.equal(retained.quality.state, 'passed'); assert.equal(retained.publication.state, 'cancelled');
  assert.equal(retained.patchDigest, accepted.request.expectedPatchDigest); assert.equal(retained.qualityDigest, accepted.request.expectedQualityDigest);
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  await mkdir(evidence, { recursive: true });
  await page.screenshot({ path: `${evidence}/task-mobile.png`, fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({ path: `${evidence}/task-desktop.png`, fullPage: true });
  assert.deepEqual(errors, []);
  await writeFile(`${evidence}/outcome.json`, JSON.stringify({ result: 'passed', scope, accepted, cancellation: { request: cancelled.request().postDataJSON(), response: await cancelled.json() }, retained, cases: ['deployed-task-read', 'real-managed-quality', 'canonical-publication-preview', 'consent', 'accepted-response-loss', 'reload-exact-retry', 'queued-publication-cancel', 'separate-retained-outcomes', 'mobile-layout'], liveGitHub: false }, null, 2));
  console.log(`task console deployed acceptance passed; evidence=${evidence}`);
} finally { await browser.close(); }
