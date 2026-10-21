import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

test('Responses fixture validates isolation profile and exposes missing usage and uncertain acceptance', async t => {
  const process = spawn(globalThis.process.execPath, [new URL('../../dev/fixtures/provider/server.mjs', import.meta.url).pathname], { env: { ...globalThis.process.env, PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => process.kill('SIGTERM'));
  const [ready] = await Promise.race([once(process.stdout, 'data'), once(process, 'exit').then(([code]) => { throw new Error(`fixture exited: ${code}`); })]);
  const endpoint = `http://127.0.0.1:${JSON.parse(ready).port}`;
  const request = input => ({ model: 'fixture-model', input: [{ role: 'user', content: input }], max_output_tokens: 64, store: false, background: false, stream: false, truncation: 'disabled' });
  const headers = { authorization: 'Bearer provider-fixture-key', 'content-type': 'application/json', 'x-client-request-id': 'fixed-call-id' };
  const send = body => fetch(`${endpoint}/v1/responses`, { method: 'POST', headers, body: JSON.stringify(body) });
  assert.equal((await send({ ...request('completed'), store: true })).status, 400);
  const completed = await (await send(request('completed'))).json();
  assert.equal(completed.status, 'completed');
  assert.equal(completed.usage.total_tokens, 20);
  const unknown = await (await send(request('fixture:no-usage'))).json();
  assert.equal(unknown.status, 'completed');
  assert.equal(unknown.usage, undefined);
  await assert.rejects(send(request('fixture:disconnect')));
  const stats = await (await fetch(`${endpoint}/stats`)).json();
  assert.equal(stats.calls, 3);
  assert.deepEqual(stats.modes, { completed: 1, 'no-usage': 1, disconnect: 1 });
  assert.deepEqual(stats.requestIds, ['fixed-call-id', 'fixed-call-id', 'fixed-call-id']);
  assert.equal(JSON.stringify(stats).includes('provider-fixture-key'), false);
});
