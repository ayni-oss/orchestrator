import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from 'vite';

test('module and CSS edits update the diagnostic UI without reloading the page', async () => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), 'ayni-live-hmr-')));
  let vite;
  let browser;
  try {
    for (const file of ['index.html', 'main.mjs', 'style.css']) {
      await writeFile(join(directory, file), await readFile(new URL(file, import.meta.url)));
    }
    vite = await createServer({
      configFile: fileURLToPath(new URL('./vite.config.mjs', import.meta.url)),
      root: directory,
      optimizeDeps: { noDiscovery: true, include: [] },
      server: { fs: { allow: [directory] } },
    });
    await vite.listen();
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    page.on('pageerror', (error) => console.error('HMR page error:', error.message));
    page.on('response', (response) => { if (response.status() >= 400) console.error('HMR response:', response.status(), response.url()); });
    await page.goto('http://127.0.0.1:5173');
    await page.waitForFunction(() => document.querySelector('#hmr')?.textContent.includes('revision 1'), null, { timeout: 10_000 });
    await page.evaluate(() => { window.liveReloadSentinel = 'retained'; });
    const main = await readFile(join(directory, 'main.mjs'), 'utf8');
    await writeFile(join(directory, 'main.mjs'), main.replace('Frontend loaded', 'Frontend updated'));
    await page.waitForFunction(() => document.querySelector('#hmr')?.textContent.includes('Frontend updated · module revision 2'));
    assert.equal(await page.evaluate(() => window.liveReloadSentinel), 'retained');
    await writeFile(join(directory, 'style.css'), ':root { background: rgb(12, 34, 56); }');
    await page.waitForFunction(() => getComputedStyle(document.documentElement).backgroundColor === 'rgb(12, 34, 56)');
    assert.equal(await page.evaluate(() => window.liveReloadSentinel), 'retained');
  } finally {
    if (browser) await browser.close();
    if (vite) await vite.close();
    await rm(directory, { recursive: true });
  }
});
