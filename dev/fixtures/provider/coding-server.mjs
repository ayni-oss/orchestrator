// Credential-free coding Responses fixture: ask once, then use the exact reviewed answer.
import http from 'node:http';
const stats = { calls: 0, questions: 0, continuations: 0, requestIds: [], answeredQuestionIds: [] };
const server = http.createServer(async (request, response) => {
  const send = (status, body) => response.writeHead(status, { 'Content-Type': 'application/json' }).end(JSON.stringify(body));
  if (request.method === 'GET' && request.url === '/health') return send(200, { result: 'ready' });
  if (request.method === 'GET' && request.url === '/stats') return send(200, stats);
  if (request.method !== 'POST' || request.url !== '/v1/responses') return send(404, {});
  if (request.headers.authorization !== 'Bearer provider-fixture-key') return send(401, {});
  let text = '';
  for await (const chunk of request) { text += chunk; if (Buffer.byteLength(text) > 256 * 1024) return send(413, {}); }
  let body; let task;
  try { body = JSON.parse(text); task = JSON.parse(body.input?.[0]?.content); } catch { return send(400, {}); }
  if (body.model !== 'fixture-model' || body.store !== false || body.background !== false || body.stream !== false || body.truncation !== 'disabled' || body.tool_choice !== 'required' || body.parallel_tool_calls !== false || !['propose_file_changes', 'request_task_recovery'].every(name => body.tools?.some(tool => tool.name === name))) return send(400, {});
  let name; let args;
  if (!task.recovery) {
    name = 'request_task_recovery';
    args = { question: 'May this fixture retain the original test and append a reviewed comment?', explanation: 'An explicit answer and confirmation are required before another bounded coding segment.', action: 'continue', additional_seconds: 300, additional_tokens: 10000 };
    stats.questions++;
  } else {
    const answers = task.recovery.answers;
    const file = task.files?.find(file => file.path === 'src/lib.rs');
    if (task.recovery.action !== 'continue' || answers?.length !== 1 || answers[0].answer !== 'Retain the original test and append only the reviewed fixture comment.' || !answers[0].question_id || !file) return send(400, {});
    stats.continuations++; stats.answeredQuestionIds.push(answers[0].question_id);
    name = 'propose_file_changes';
    args = { changes: [{ path: file.path, content: `${file.content}\n// Continued after explicit review of the worker question.\n` }] };
  }
  stats.calls++; stats.requestIds.push(request.headers['x-client-request-id'] || 'missing');
  return send(200, { id: `resp_coding_fixture_${stats.calls}`, object: 'response', model: 'fixture-model', status: 'completed', output: [{ id: `fc_coding_fixture_${stats.calls}`, type: 'function_call', status: 'completed', call_id: `call_coding_fixture_${stats.calls}`, name, arguments: JSON.stringify(args) }], usage: { input_tokens: 12, output_tokens: 8, total_tokens: 20 } });
});
server.listen(Number(process.env.PORT || 8080), '0.0.0.0', () => console.log(JSON.stringify({ port: server.address().port })));
process.on('SIGTERM', () => server.close());
process.on('SIGINT', () => server.close());
