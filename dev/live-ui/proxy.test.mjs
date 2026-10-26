import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer as createHttpServer, request as httpRequest } from 'node:http';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

test('live frontend proxies canonical requests only after checking the browser boundary', async () => {
  const received = [];
  const upstream = createHttpServer((request, response) => {
    received.push({ url: request.url, headers: request.headers });
    response.setHeader('Content-Type', request.url === '/watch' ? 'text/event-stream' : 'application/json');
    response.end(request.url === '/watch' ? 'event: frame\ndata: {}\n\n' : JSON.stringify({ workspaces: [] }));
  });
  let vite;
  try {
    await new Promise((resolve, reject) => {
      upstream.once('error', reject);
      upstream.listen(0, '127.0.0.1', resolve);
    });
    const target = `http://127.0.0.1:${upstream.address().port}`;
    vite = await createServer({
      configFile: fileURLToPath(new URL('./vite.config.mjs', import.meta.url)),
      optimizeDeps: { noDiscovery: true, include: [] },
      server: { proxy: { '/api': { target }, '/watch': { target } } },
    });
    await vite.listen();
    const request = {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Origin: 'http://127.0.0.1:5173',
        'X-Console-Request': '1',
      },
      body: '{}',
    };
    const valid = await fetch('http://127.0.0.1:5173/api/ListWorkspaces', request);
    assert.equal(valid.status, 200);
    assert.deepEqual(await valid.json(), { workspaces: [] });
    assert.equal(received[0].headers.origin, 'http://127.0.0.1:3000');
    assert.equal(received[0].headers.host, `127.0.0.1:${upstream.address().port}`);
    const foreign = await fetch('http://127.0.0.1:5173/api/ListWorkspaces', {
      ...request, headers: { ...request.headers, Origin: 'https://attacker.example' },
    });
    assert.equal(foreign.status, 403);
    assert.equal(received.length, 1);
    const rebinding = await new Promise((resolve, reject) => {
      const outgoing = httpRequest('http://127.0.0.1:5173/api/ListWorkspaces', {
        method: 'POST', headers: { ...request.headers, Host: 'attacker.example:5173' },
      }, (response) => { response.resume(); resolve(response.statusCode); });
      outgoing.on('error', reject);
      outgoing.end('{}');
    });
    assert.equal(rebinding, 403);
    assert.equal(received.length, 1);
    const watch = await fetch('http://127.0.0.1:5173/watch');
    assert.equal(await watch.text(), 'event: frame\ndata: {}\n\n');
    const client = await fetch('http://127.0.0.1:5173/@vite/client');
    assert.equal(client.status, 200);
    const module = await fetch('http://127.0.0.1:5173/main.mjs');
    assert.match(await module.text(), /import\.meta\.hot/);
    const status = await fetch('http://127.0.0.1:5173/__live/status');
    assert.equal(status.status, 200);
    assert.equal(typeof (await status.json()).stale, 'boolean');
  } finally {
    if (vite) await vite.close();
    upstream.closeAllConnections();
    await new Promise((resolve) => upstream.close(resolve));
  }
});
