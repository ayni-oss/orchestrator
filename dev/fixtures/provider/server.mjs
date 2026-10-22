// Deterministic Responses fixture. This server never contacts an external provider.
import http from 'node:http';
const port = Number(process.env.PORT || 8080);
const delay = Number(process.env.PROVIDER_FIXTURE_DELAY_MS || 5000);
const counters = { calls: 0, modes: {}, requestIds: [], toolContinuations: 0 };
const toolNames = ['read_workspace_instructions', 'read_project_status', 'read_conversation_history'];
const validTools = tools => tools === undefined || (Array.isArray(tools) && [3, 4].includes(tools.length) && new Set(tools.map(tool => tool.name)).size === tools.length && toolNames.every(name => tools.some(tool => tool.name === name)) && tools.every(tool => {
  if (tool.type !== 'function' || tool.strict !== true || tool.parameters?.type !== 'object' || tool.parameters?.additionalProperties !== false) return false;
  if (toolNames.includes(tool.name)) return Object.keys(tool.parameters.properties || {}).length === 0 && Array.isArray(tool.parameters.required) && tool.parameters.required.length === 0;
  return tool.name === 'propose_plan' && ['content', 'assumptions', 'evidence'].every(name => tool.parameters.properties?.[name] && tool.parameters.required?.includes(name));
}));
const proposal = () => ({
  content: {
    title: 'Fixture plan <script>window.proposalInjected=true</script>',
    objective: 'Preserve the discussed scope for explicit review.',
    scope: 'Only the originating conversation.', out_of_scope: 'Approval and coding dispatch.',
    constraints: ['Retain immutable version identity.'], unresolved_questions: ['Confirm the requested scope.'],
    tasks: [{ key: 'review', objective: 'Review the retained proposal.', scope: 'The discussed change.', out_of_scope: 'Execution.', acceptance_criteria: ['Review origin and assumptions.'], repository: 'declared/example', source_ref: '', execution_limits: 'Await explicit approval.', unresolved_inputs: ['Verify repository and source before execution.'] }],
  },
  assumptions: ['The user wants a reviewable proposal.'],
  evidence: ['Conversation discussion; repository contents have not been inspected.'],
});
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
  if (body.model !== 'fixture-model' || body.store !== false || body.background !== false || body.stream !== false || body.truncation !== 'disabled' || !Array.isArray(body.input) || !Number.isInteger(body.max_output_tokens) || body.max_output_tokens < 1 || body.previous_response_id || body.conversation || !validTools(body.tools)) {
    response.writeHead(400).end(); return;
  }
  const content = body.input.filter(item => item.role === 'user').at(-1)?.content || '';
  const mode = ['proposal-malformed', 'proposal-scope', 'proposal-delay', 'proposal-mixed', 'proposal', 'no-usage', 'disconnect', 'delay', 'tool-scope', 'tool-limit', 'tool-args', 'tool-unknown', 'tool-restart', 'tools'].find(value => content.includes(`fixture:${value}`)) || 'completed';
  counters.calls++;
  const sequence = counters.calls;
  counters.modes[mode] = (counters.modes[mode] || 0) + 1;
  counters.requestIds.push(request.headers['x-client-request-id'] || 'missing');
  if (mode === 'disconnect') { response.destroy(); return; }
  if (mode === 'delay' || mode === 'proposal-delay') await new Promise(resolve => setTimeout(resolve, delay));
  const outputs = body.input.filter(item => item.type === 'function_call_output');
  if (mode === 'tool-restart' && outputs.length) await new Promise(resolve => setTimeout(resolve, 30000));
  if (mode === 'tools' && outputs.length) {
    const expected = new Map(body.input.filter(item => item.type === 'function_call').map(item => [item.call_id, item.name]));
    if (outputs.length !== 3 || new Set(outputs.map(item => item.call_id)).size !== 3 || outputs.some(item => !expected.has(item.call_id) || typeof item.output !== 'string' || Buffer.byteLength(item.output) > 32768)) { response.writeHead(400).end(); return; }
    try {
      for (const item of outputs) {
        const value = JSON.parse(item.output);
        switch (expected.get(item.call_id)) {
          case 'read_workspace_instructions': if (typeof value.instructions !== 'string' || !value.revision_id) throw new Error(); break;
          case 'read_project_status': if (typeof value.objective !== 'string' || !value.project_id || !value.status) throw new Error(); break;
          case 'read_conversation_history': if (!Array.isArray(value.records) || !value.records.some(record => record.role === 'user' && record.content.includes('fixture:tools'))) throw new Error(); break;
          default: throw new Error();
        }
      }
    } catch { response.writeHead(400).end(); return; }
    counters.toolContinuations++;
  }
  const toolMode = mode.startsWith('tool-') || mode === 'tools';
  const result = {
    id: `resp_fixture_${sequence}`, object: 'response', model: 'fixture-model', status: 'completed',
    output: [{ id: `msg_fixture_${sequence}`, type: 'message', role: 'assistant', status: 'completed', content: [{ type: 'output_text', text: `Provider fixture response ${sequence}.`, annotations: [] }] }],
  };
  if (toolMode && outputs.length === 0) {
    if (!body.tools) { response.writeHead(400).end(); return; }
    const names = mode === 'tool-limit' ? Array.from({ length: 9 }, () => toolNames[0]) : mode === 'tools' ? toolNames : [mode === 'tool-unknown' ? 'shell' : toolNames[0]];
    result.output = names.map((name, index) => ({ id: `fc_fixture_${sequence}_${index}`, type: 'function_call', status: 'completed', call_id: `call_fixture_${sequence}_${index}`, name, arguments: mode === 'tool-scope' ? '{"conversation_id":"another-conversation"}' : mode === 'tool-args' ? JSON.stringify({ payload: 'x'.repeat(4096) }) : '{}' }));
  }
  if (mode.startsWith('proposal')) {
    if (!body.tools?.some(tool => tool.name === 'propose_plan')) { response.writeHead(400).end(); return; }
    const value = proposal();
    if (mode === 'proposal-malformed') value.content.tasks[0].key = 'invalid task key';
    if (mode === 'proposal-scope') value.conversation_id = 'another-conversation';
    result.output = [{ id: `fc_fixture_${sequence}`, type: 'function_call', status: 'completed', call_id: `proposal_fixture_${sequence}`, name: 'propose_plan', arguments: JSON.stringify(value) }];
    if (mode === 'proposal-mixed') result.output.push({ id: `fc_fixture_${sequence}_read`, type: 'function_call', status: 'completed', call_id: `read_fixture_${sequence}`, name: toolNames[0], arguments: '{}' });
  }
  if (mode !== 'no-usage') result.usage = { input_tokens: 12, output_tokens: 8, total_tokens: 20 };
  response.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify(result));
});
server.listen(port, '0.0.0.0', () => console.log(JSON.stringify({ port: server.address().port })));
process.on('SIGTERM', () => server.close());
process.on('SIGINT', () => server.close());
