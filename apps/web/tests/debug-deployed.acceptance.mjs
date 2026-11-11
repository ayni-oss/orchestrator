// Rust owns deployed fixtures. This test owns only browser assertions.
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
const sourceId = process.env.AYNI_DEBUG_SOURCE_ID;
assert.ok(sourceId, 'run through cargo xtask test debug');
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage();
  page.setDefaultTimeout(20000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('http://127.0.0.1:5174/debug/logs');
  await page.getByLabel('Source ID', { exact: true }).fill(sourceId);
  await page.getByRole('button', { name: 'Apply filters', exact: true }).click();
  await page.getByRole('button', { name: 'Follow live', exact: true }).click();
  await page.getByRole('region', { name: 'Execution logs' }).getByText('<img src=x onerror=', { exact: false }).first().waitFor();
  await page.getByRole('button', { name: 'Pause', exact: true }).click();
  assert.equal(await page.evaluate(() => globalThis.__rawExecuted), undefined);
  assert.equal(await page.getByRole('region', { name: 'Execution logs' }).locator('img').count(), 0);
  assert.equal(await page.evaluate(() => localStorage.length), 0);
  assert.deepEqual(errors, []);
} finally { await browser.close(); }
