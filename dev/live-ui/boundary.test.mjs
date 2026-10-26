import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { localRequest, readStatus, uiAuthority, uiOrigin } from './boundary.mjs';

test('proxy accepts only the exact local authority and browser origin', () => {
  assert.equal(localRequest({ host: uiAuthority }), true);
  assert.equal(localRequest({ host: uiAuthority, origin: uiOrigin, 'sec-fetch-site': 'same-origin' }), true);
  for (const headers of [
    { host: 'attacker.example:5173' },
    { host: uiAuthority, origin: 'https://attacker.example' },
    { host: uiAuthority, origin: 'null' },
    { host: uiAuthority, 'sec-fetch-site': 'cross-site' },
    { host: uiAuthority, 'sec-fetch-site': 'same-site' },
    { host: 'localhost:5173' },
  ]) assert.equal(localRequest(headers), false);
});

test('status reports stale, missing and invalid observations explicitly', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'ayni-live-ui-'));
  const path = join(directory, 'status.json');
  try {
    assert.equal((await readStatus(path)).stale, true);
    await writeFile(path, JSON.stringify({ stage: 'build_failed', serving: 'previous' }));
    assert.deepEqual((await readStatus(path)).status, { stage: 'build_failed', serving: 'previous' });
    assert.equal((await readStatus(path)).stale, false);
    await utimes(path, 1, 1);
    assert.equal((await readStatus(path)).stale, true);
    await writeFile(path, 'malformed');
    assert.equal((await readStatus(path)).status, null);
    await writeFile(path, ' '.repeat(65537));
    assert.equal((await readStatus(path)).status, null);
  } finally {
    await rm(directory, { recursive: true });
  }
});
