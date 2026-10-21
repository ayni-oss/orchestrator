// Deterministic Responses fixture. This server never contacts an external provider.
import http from 'node:http';
const port = Number(process.env.PORT || 8080);
const delay = Number(process.env.PROVIDER_FIXTURE_DELAY_MS || 5000);
const counters = { calls: 0, modes: {}, requestIds: [] };
const server = http.createServer(async (request, response) => {
  if (request.url === '/health') { response.writeHead(200).end('ok'); return; }
  if (request.method === 'GET' && request.url === '/stats') {
    response.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify(counters)); return;
  }
  if (request.method !== 'POST' || request.url !== '/v1/responses') { response.writeHead(404).end(); return; }
  if (request.headers.authorization !== 'Bearer provider-fixture-key') { response.writeHead(401).end(); return; }
  let bytes = 0; const chunks = [];
  for await (const chunk of request) {
    bytes += chunk.length;
    if (bytes > 256 * 1024) { response.writeHead(413).end(); return; }
    chunks.push(chunk);
  }
  let body;
  try { body = JSON.parse(Buffer.concat(chunks).toString()); } catch { response.writeHead(400).end(); return; }
  if (body.model !== 'fixture-model' || body.store !== false || body.background !== false || body.stream !== false || body.truncation !== 'disabled' || !Array.isArray(body.input) || !Number.isInteger(body.max_output_tokens) || body.max_output_tokens < 1 || body.previous_response_id || body.conversation || body.tools) {
    response.writeHead(400).end(); return;
  }
  const content = body.input.filter(item => item.role === 'user').at(-1)?.content || '';
  const mode = ['no-usage', 'disconnect', 'delay'].find(value => content.includes(`fixture:${value}`)) || 'completed';
  counters.calls++;
  const sequence = counters.calls;
  counters.modes[mode] = (counters.modes[mode] || 0) + 1;
  counters.requestIds.push(request.headers['x-client-request-id'] || 'missing');
  if (mode === 'disconnect') { response.destroy(); return; }
  if (mode === 'delay') await new Promise(resolve => setTimeout(resolve, delay));
  const result = {
    id: `resp_fixture_${sequence}`, object: 'response', model: 'fixture-model', status: 'completed',
    output: [{ id: `msg_fixture_${sequence}`, type: 'message', role: 'assistant', status: 'completed', content: [{ type: 'output_text', text: `Provider fixture response ${sequence}.`, annotations: [] }] }],
  };
  if (mode !== 'no-usage') result.usage = { input_tokens: 12, output_tokens: 8, total_tokens: 20 };
  response.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify(result));
});
server.listen(port, '0.0.0.0', () => console.log(JSON.stringify({ port: server.address().port })));
process.on('SIGTERM', () => server.close());
process.on('SIGINT', () => server.close());
