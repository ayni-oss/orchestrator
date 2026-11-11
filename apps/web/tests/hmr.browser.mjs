import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { chromium } from 'playwright';
import { origin } from './ui-fixture.mjs';

test('the singleton application retains browser state through component and CSS HMR', {timeout:120000}, async () => {
  const source = new URL('../src/lib/components/hierarchy-overview.svelte', import.meta.url);
  const css = new URL('../src/app.css', import.meta.url);
  const originalSource = await readFile(source, 'utf8');
  const originalCss = await readFile(css, 'utf8');
  const title = 'title={data.workspace?.displayName ?? label}';
  assert.ok(originalSource.includes(title), 'home page title binding must exist');
  const browser = await chromium.launch({headless:true});
  try {
    const page = await browser.newPage();
    await page.goto(origin);
    await page.waitForFunction(() => document.documentElement.dataset.hydrated === 'true');
    await page.evaluate(() => { window.hmrSentinel = 'retained'; });
    await writeFile(source, originalSource.replace(title, 'title="Updated projects"'));
    await page.getByRole('heading', {name:'Updated projects',exact:true}).waitFor();
    assert.equal(await page.evaluate(() => window.hmrSentinel), 'retained');
    await writeFile(css, originalCss + '\n:root { --primary: rgb(12, 34, 56); }\n');
    await page.waitForFunction(() => getComputedStyle(document.documentElement).getPropertyValue('--primary').trim() === 'rgb(12, 34, 56)');
    assert.equal(await page.evaluate(() => window.hmrSentinel), 'retained');
  } finally {
    await writeFile(source, originalSource);
    await writeFile(css, originalCss);
    await browser.close();
  }
});
