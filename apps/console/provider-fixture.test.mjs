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

test('Responses fixture validates scoped reads and validates tool continuations', async t => {
  const fixtureProcess = spawn(globalThis.process.execPath, [new URL('../../dev/fixtures/provider/server.mjs', import.meta.url).pathname], { env: { ...globalThis.process.env, PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => fixtureProcess.kill('SIGTERM'));
  const [ready] = await Promise.race([once(fixtureProcess.stdout, 'data'), once(fixtureProcess, 'exit').then(([code]) => { throw new Error(`fixture exited: ${code}`); })]);
  const endpoint = `http://127.0.0.1:${JSON.parse(ready).port}`;
  const names = ['read_workspace_instructions', 'read_project_status', 'read_conversation_history'];
  const tools = names.map(name => ({ type: 'function', name, strict: true, parameters: { type: 'object', properties: {}, required: [], additionalProperties: false } }));
  const request = content => ({ model: 'fixture-model', input: [{ role: 'user', content }], tools, max_output_tokens: 512, store: false, background: false, stream: false, truncation: 'disabled' });
  const send = body => fetch(`${endpoint}/v1/responses`, { method: 'POST', headers: { authorization: 'Bearer provider-fixture-key', 'content-type': 'application/json', 'x-client-request-id': 'scoped-tool-fixture' }, body: JSON.stringify(body) });
  assert.equal((await send({ ...request('fixture:tools'), tools: [{ type: 'web_search' }] })).status, 400);
  assert.equal((await send({ ...request('fixture:tools'), tools: [...tools, tools[0]] })).status, 400);
  assert.equal((await send({ ...request('fixture:tools'), tools: tools.map(tool => ({ ...tool, parameters: { ...tool.parameters, additionalProperties: true } })) })).status, 400);
  const prepared = request('fixture:tools');
  const response = await (await send(prepared)).json();
  assert.deepEqual(response.output.map(call => call.name), names);
  const values = [
    { revision_id: 'revision', instructions: 'frozen' },
    { project_id: 'project', objective: 'factual objective', status: 'active', context_revision: '7' },
    { records: [{ message_id: 'input', sequence: 1, role: 'user', content: 'fixture:tools' }] },
  ];
  const outputs = response.output.map((call, index) => ({ type: 'function_call_output', call_id: call.call_id, output: JSON.stringify(values[index]) }));
  const continuation = { ...prepared, input: [...prepared.input, ...response.output, ...outputs] };
  assert.equal((await send({ ...continuation, input: [...prepared.input, ...response.output, { ...outputs[0], call_id: 'foreign' }, ...outputs.slice(1)] })).status, 400);
  const completed = await (await send(continuation)).json();
  assert.equal(completed.output[0].type, 'message');
  assert.match(completed.output[0].content[0].text, /Provider fixture response/);
  const stats = await (await fetch(`${endpoint}/stats`)).json();
  assert.equal(stats.toolContinuations, 1);
  assert.equal(JSON.stringify(stats).includes('factual objective'), false, 'fixture statistics do not retain tool output');
  const overLimit = await (await send(request('fixture:tool-limit'))).json();
  assert.equal(overLimit.output.length, 9);
  const foreignScope = await (await send(request('fixture:tool-scope'))).json();
  assert.deepEqual(JSON.parse(foreignScope.output[0].arguments), { conversation_id: 'another-conversation' });
  const oversized = await (await send(request('fixture:tool-args'))).json();
  assert.ok(Buffer.byteLength(oversized.output[0].arguments) > 4096);
  const unknown = await (await send(request('fixture:tool-unknown'))).json();
  assert.equal(unknown.output[0].name, 'shell');
});


test('Responses fixture emits bounded plan proposals and deterministic malformed/scope attacks', async t => {
  const fixtureProcess = spawn(globalThis.process.execPath, [new URL('../../dev/fixtures/provider/server.mjs', import.meta.url).pathname], { env: { ...globalThis.process.env, PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => fixtureProcess.kill('SIGTERM'));
  const [ready] = await Promise.race([once(fixtureProcess.stdout, 'data'), once(fixtureProcess, 'exit').then(([code]) => { throw new Error(`fixture exited: ${code}`); })]);
  const endpoint = `http://127.0.0.1:${JSON.parse(ready).port}`;
  const tools = ['read_workspace_instructions', 'read_project_status', 'read_conversation_history'].map(name => ({ type: 'function', name, strict: true, parameters: { type: 'object', properties: {}, required: [], additionalProperties: false } }));
  tools.push({ type: 'function', name: 'propose_plan', strict: true, parameters: { type: 'object', properties: { content: { type: 'object' }, assumptions: { type: 'array' }, evidence: { type: 'array' } }, required: ['content', 'assumptions', 'evidence'], additionalProperties: false } });
  const send = mode => fetch(`${endpoint}/v1/responses`, { method: 'POST', headers: { authorization: 'Bearer provider-fixture-key', 'content-type': 'application/json' }, body: JSON.stringify({ model: 'fixture-model', input: [{ role: 'user', content: `fixture:${mode}` }], tools, max_output_tokens: 2048, store: false, background: false, stream: false, truncation: 'disabled' }) });
  const valid = await (await send('proposal')).json();
  assert.equal(valid.output.length, 1);
  assert.equal(valid.output[0].name, 'propose_plan');
  const args = JSON.parse(valid.output[0].arguments);
  assert.ok(Buffer.byteLength(valid.output[0].arguments) < 40960);
  assert.ok(args.assumptions.length && args.evidence.length);
  assert.ok(args.content.unresolved_questions.length);
  assert.equal(args.content.tasks[0].repository, 'declared/example');
  assert.equal(args.conversation_id, undefined, 'valid proposal cannot select scope');
  const malformed = JSON.parse((await (await send('proposal-malformed')).json()).output[0].arguments);
  assert.equal(malformed.content.tasks[0].key, 'invalid task key');
  const foreign = JSON.parse((await (await send('proposal-scope')).json()).output[0].arguments);
  assert.equal(foreign.conversation_id, 'another-conversation');
  assert.equal((await (await send('proposal-mixed')).json()).output.length, 2);
});

test('Anthropic fixture validates selected headers, tools, cancellation delay and ambiguous transport', async t => {
  const fixtureProcess = spawn(globalThis.process.execPath, [new URL('../../dev/fixtures/provider/server.mjs', import.meta.url).pathname], { env: { ...globalThis.process.env, PORT:'0', PROVIDER_FIXTURE_DELAY_MS:'5' }, stdio:['ignore','pipe','pipe'] });
  t.after(()=>fixtureProcess.kill('SIGTERM'));
  const [ready]=await Promise.race([once(fixtureProcess.stdout,'data'),once(fixtureProcess,'exit').then(([code])=>{throw new Error(`fixture exited: ${code}`);})]);
  const endpoint=`http://127.0.0.1:${JSON.parse(ready).port}`;
  const headers={'content-type':'application/json','x-api-key':'provider-fixture-key','anthropic-version':'2023-06-01'};
  const tools=['mcp_fixture_read','mcp_fixture_write'].map(name=>({name,input_schema:{type:'object',properties:{},required:[],additionalProperties:false}}));
  const body={model:'fixture-model',system:'frozen',messages:[{role:'user',content:[{type:'text',text:'fixture:mcp'}]}],tools,max_tokens:512,stream:false,thinking:{type:'disabled'}};
  const send=(body,selectedHeaders=headers)=>fetch(`${endpoint}/v1/messages`,{method:'POST',headers:selectedHeaders,body:JSON.stringify(body)});
  assert.equal((await send(body,{...headers,'anthropic-version':'incorrect'})).status,401);
  assert.equal((await send({...body,stream:true})).status,400);
  const first=await(await send(body)).json();
  assert.equal(first.type,'message');
  assert.equal(first.stop_reason,'tool_use');
  assert.deepEqual(first.content.map(block=>block.name),tools.map(tool=>tool.name));
  const continuation={...body,messages:[...body.messages,{role:'assistant',content:first.content},{role:'user',content:first.content.map(block=>({type:'tool_result',tool_use_id:block.id,content:'Accepted scoped fixture output.'}))}]};
  const final=await(await send(continuation)).json();
  assert.equal(final.stop_reason,'end_turn');
  assert.match(final.content[0].text,/Anthropic fixture/);
  assert.equal(final.usage.input_tokens+final.usage.output_tokens,20);
  await assert.rejects(send({...body,messages:[{role:'user',content:[{type:'text',text:'fixture:disconnect'}]}]}));
  const delayed=await(await send({...body,messages:[{role:'user',content:[{type:'text',text:'fixture:delay'}]}]})).json();
  assert.equal(delayed.stop_reason,'end_turn');
  const stats=await(await fetch(`${endpoint}/stats`)).json();
  assert.equal(stats.anthropicCalls,4);
  assert.equal(stats.toolContinuations,1);
  assert.equal(JSON.stringify(stats).includes('provider-fixture-key'),false);
});

test('generated outcome assessment prompts yield reviewable proposals through the deterministic fixture', async t => {
  const fixtureProcess = spawn(globalThis.process.execPath, [new URL('../../dev/fixtures/provider/server.mjs', import.meta.url).pathname], { env: { ...globalThis.process.env, PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(() => fixtureProcess.kill('SIGTERM'));
  const [ready] = await Promise.race([once(fixtureProcess.stdout, 'data'), once(fixtureProcess, 'exit').then(([code]) => { throw new Error(`fixture exited: ${code}`); })]);
  const endpoint = `http://127.0.0.1:${JSON.parse(ready).port}`;
  const tools = ['read_workspace_instructions', 'read_project_status', 'read_conversation_history'].map(name => ({ type: 'function', name, strict: true, parameters: { type: 'object', properties: {}, required: [], additionalProperties: false } }));
  tools.push({ type: 'function', name: 'propose_plan', strict: true, parameters: { type: 'object', properties: { content: {}, assumptions: {}, evidence: {} }, required: ['content', 'assumptions', 'evidence'], additionalProperties: false } });
  const response = await fetch(`${endpoint}/v1/responses`, { method: 'POST', headers: { authorization: 'Bearer provider-fixture-key', 'content-type': 'application/json', 'x-client-request-id': 'assessment-fixture' }, body: JSON.stringify({ model: 'fixture-model', input: [{ role: 'user', content: 'Authorized task outcome assessment. Retained accepted outcomes follow.' }], tools, max_output_tokens: 512, store: false, background: false, stream: false, truncation: 'disabled' }) });
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.output[0].name, 'propose_plan');
  assert.equal(JSON.parse(body.output[0].arguments).content.tasks.length, 1);
  const stats = await (await fetch(`${endpoint}/stats`)).json();
  assert.equal(stats.modes.assessment, 1);
  assert.equal(stats.calls, 1);
});
