// Run against the deployed deterministic provider fixture and local console.
// This never supplies a real provider credential or contacts OpenAI.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
const endpoint = process.env.CONSOLE_URL || 'http://127.0.0.1:3000';
const fixture = process.env.PROVIDER_FIXTURE_URL || 'http://127.0.0.1:18080';
const namespace = process.env.AYNI_ORCHESTRATOR_NAMESPACE || 'ayni-orchestrator-dev';
for (const value of [endpoint, fixture]) {
  const url = new URL(value);
  assert.equal(url.hostname, '127.0.0.1', 'provider acceptance requires local forwarded services');
  assert.equal(url.protocol, 'http:');
}
assert.match(namespace, /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/);
assert.ok(namespace.length <= 54);
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
const stats = async () => (await fetch(`${fixture}/stats`)).json();
const before = await stats();
const gateway = async (method, body) => {
  const result = await page.evaluate(async ({ method, body }) => {
    const response = await fetch(`/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  }, { method, body });
  assert.equal(result.status, 200, `${method}: ${JSON.stringify(result.body)}`);
  return result.body;
};
const waitFor = async (read, predicate, label) => {
  const until = Date.now() + 120000;
  while (Date.now() < until) {
    const value = await read();
    if (predicate(value)) return value;
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`Timed out: ${label}`);
};
const key = () => crypto.randomUUID();
let scope;
const calls = async () => (await gateway('ListProviderCalls', scope)).calls || [];
const snapshot = async () => (await gateway('GetConversationSnapshot', scope)).snapshot;
const send = async content => (await gateway('SendMessage', { ...scope, content, idempotencyKey: key() })).receipt;
const terminal = async turnId => waitFor(snapshot, value => (value.recentTerminalTurns || []).some(turn => turn.id === turnId), 'terminal acceptance');
try {
  await page.goto(endpoint);
  const workspace = (await gateway('CreateWorkspace', { idempotencyKey: key(), displayName: `Provider acceptance ${Date.now()}` })).workspace;
  // This constant is a public fixture credential. It has no external capability.
  execFileSync('kubectl', ['--context', 'docker-desktop', '-n', `${namespace}-sessions`, 'apply', '-f', '-'], {
    input: JSON.stringify({ apiVersion: 'v1', kind: 'Secret', metadata: { name: `provider-openai-${workspace.id.replaceAll('-', '')}` }, type: 'Opaque', stringData: { 'api-key': 'provider-fixture-key' } }),
    stdio: ['pipe', 'ignore', 'pipe'],
  });
  const configuration = (await gateway('GetAgentConfiguration', { workspaceId: workspace.id })).configuration;
  const project = (await gateway('CreateProject', { workspaceId: workspace.id, displayName: 'Provider fixture', goal: 'Verify retained provider conversations', idempotencyKey: key() })).project;
  const conversation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: 'Provider recovery', idempotencyKey: key() })).conversation;
  scope = { workspaceId: workspace.id, projectId: project.id, conversationId: conversation.id };
  const scripted = await send('This turn was admitted before the provider cutover.');
  await terminal(scripted.turnId);
  assert.equal((await stats()).calls, before.calls);
  await gateway('UpdateAgentConfiguration', { idempotencyKey: key(), expectedRevisionId: configuration.revisionId, configuration: { workspaceId: workspace.id, provider: 'openai', model: 'fixture-model', instructions: 'Use only this frozen conversation context.', maxInputTokens: configuration.maxInputTokens, maxOutputTokens: configuration.maxOutputTokens, maxTotalTokens: configuration.maxTotalTokens, idleTimeoutSeconds: configuration.idleTimeoutSeconds } });
  assert.equal((await gateway('GetTurnInputs', { ...scope, turnId: scripted.turnId })).configuration.provider, 'scripted');
  await page.evaluate(scope => sessionStorage.setItem('selection', JSON.stringify(scope)), scope);
  await page.reload();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  const first = await send('fixture:completed');
  await terminal(first.turnId);
  const firstCall = (await calls()).find(call => call.turnId === first.turnId);
  assert.ok(firstCall?.responseId);
  assert.equal(firstCall.provider, 'openai');
  assert.equal(firstCall.model, 'fixture-model');
  assert.equal(firstCall.inputTokens, '12');
  assert.equal(firstCall.outputTokens, '8');
  assert.equal(firstCall.totalTokens, '20');
  await page.locator('#refresh-provider-calls').click();
  await page.getByText('openai / fixture-model', { exact: true }).first().waitFor();
  await page.getByText('Reported usage: Input 12 · output 8 · total 20', { exact: true }).waitFor();
  await page.reload();
  await page.getByText('Provider fixture response ', { exact: false }).first().waitFor();
  const afterReload = await stats();
  assert.equal(afterReload.calls, before.calls + 1, 'reload must not create another provider call');
  const second = await send('fixture:no-usage');
  await terminal(second.turnId);
  const unknown = (await calls()).find(call => call.turnId === second.turnId);
  assert.ok(unknown?.responseId);
  assert.equal(unknown.inputTokens, undefined);
  assert.equal(unknown.outputTokens, undefined);
  assert.equal(unknown.totalTokens, undefined);
  assert.notEqual(unknown.reservedTotalTokens, '0', 'unknown usage retains its reservation');
  await page.locator('#refresh-provider-calls').click();
  await page.getByText('Reported usage: Input Unknown · output Unknown · total Unknown', { exact: true }).waitFor();
  const uncertainTurn = await send('fixture:disconnect');
  await waitFor(calls, values => values.some(call => call.turnId === uncertainTurn.turnId && call.state === 'uncertain'), 'uncertain call');
  const uncertainCalls = (await stats()).calls;
  const queued = await send('This queued turn must never call the fixture.');
  const queuedCancellation = await gateway('CancelTurn', { ...scope, turnId: queued.turnId, idempotencyKey: key() });
  assert.equal(queuedCancellation.state, 'cancelled');
  const queuedTerminal = await terminal(queued.turnId);
  assert.equal(queuedTerminal.recentTerminalTurns.find(turn => turn.id === queued.turnId).safeFailureReason, 'cancelled');
  assert.equal((await stats()).calls, uncertainCalls, 'queued cancellation must prevent a provider request');
  await page.reload();
  await page.getByText(/The external outcome is unknown/).waitFor();
  assert.equal((await stats()).calls, uncertainCalls, 'uncertain replay must not resend');
  // Lose the cancellation response after the durable command has completed.
  // The tab must retain the original key, including after reload.
  let cancelledCommand;
  await page.route('**/api/CancelTurn', async route => {
    cancelledCommand = route.request().postDataJSON();
    const accepted = await route.fetch();
    assert.equal(accepted.status(), 200);
    await route.abort('connectionfailed');
  }, { times: 1 });
  await page.locator(`[data-cancel-turn="${uncertainTurn.turnId}"]`).click();
  await page.locator('#pending').waitFor({ state: 'visible' });
  await waitFor(() => page.evaluate(() => JSON.parse(sessionStorage.getItem('pending'))), value => value?.method === 'CancelTurn', 'retained cancel retry');
  await page.reload();
  await page.locator('#pending').waitFor({ state: 'visible' });
  const retryResponse = page.waitForResponse(response => new URL(response.url()).pathname === '/api/CancelTurn');
  await page.locator('#retry').click();
  const replay = await retryResponse;
  assert.equal(replay.status(), 200);
  assert.deepEqual(replay.request().postDataJSON(), cancelledCommand);
  await page.locator('#pending').waitFor({ state: 'hidden' });
  const cancelled = (await calls()).find(call => call.turnId === uncertainTurn.turnId);
  assert.equal(cancelled.cancellationRequested, true);
  assert.equal(cancelled.state, 'uncertain');
  assert.equal((await stats()).calls, uncertainCalls, 'cancellation must not resend an uncertain call');
  const retained = await snapshot();
  assert.equal((retained.recentMessages || []).filter(message => message.role === 'MESSAGE_ROLE_ASSISTANT').length, 3);
  // A separate conversation tests cancellation while network I/O is in flight.
  const uncertainScope = scope;
  const delayedConversation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: 'Provider cancellation race', idempotencyKey: key() })).conversation;
  scope = { ...scope, conversationId: delayedConversation.id };
  const delayed = await send('fixture:delay');
  await waitFor(calls, values => values.some(call => call.turnId === delayed.turnId && call.state === 'sending'), 'sending boundary');
  const delayedCancellation = await gateway('CancelTurn', { ...scope, turnId: delayed.turnId, idempotencyKey: key() });
  assert.equal(delayedCancellation.cancellationRequested, true);
  const delayedCall = await waitFor(calls, values => values.some(call => call.turnId === delayed.turnId && call.cancellationRequested && ['uncertain', 'cancelled', 'completed'].includes(call.state)), 'in-flight cancellation evidence');
  assert.ok(delayedCall.find(call => call.turnId === delayed.turnId).cancellationRequested);
  assert.equal((await snapshot()).recentMessages.filter(message => message.role === 'MESSAGE_ROLE_ASSISTANT').length, 0, 'cancellation suppresses a later assistant result');
  scope = uncertainScope;
  assert.deepEqual(errors, []);
  if (process.env.CONSOLE_EVIDENCE_DIRECTORY) {
    await mkdir(process.env.CONSOLE_EVIDENCE_DIRECTORY, { recursive: true });
    await page.screenshot({ path: `${process.env.CONSOLE_EVIDENCE_DIRECTORY}/provider-console.png`, fullPage: true });
  }
  console.log(JSON.stringify({ outcome: 'passed', scope, checks: ['scripted-to-openai-cutover', 'responses-request-profile', 'retained-response-usage', 'reload-no-resend', 'unknown-usage-reservation', 'uncertainty-no-resend', 'queued-cancellation-no-call', 'in-flight-cancellation', 'cancellation-receipt-replay', 'exactly-once-visible-results'], calls: (await stats()).calls - before.calls }));
} finally { await browser.close(); }
