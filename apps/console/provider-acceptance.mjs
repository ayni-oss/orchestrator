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
const kube = (...arguments_) => execFileSync('kubectl', ['--context', 'docker-desktop', '-n', `${namespace}-sessions`, ...arguments_], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
const workerIdentity = session => {
  const name = `session-${session.id.replaceAll('-', '')}-${session.generation}`;
  const pod = JSON.parse(kube('get', 'pod', name, '-o', 'json'));
  assert.equal(pod.metadata.labels['build.ayni/session-id'], session.id);
  assert.equal(pod.metadata.labels['build.ayni/session-generation'], session.generation);
  const pvcName = pod.spec.volumes.find(volume => volume.name === 'ledger').persistentVolumeClaim.claimName;
  const pvc = JSON.parse(kube('get', 'pvc', pvcName, '-o', 'json'));
  return { name, podUid: pod.metadata.uid, pvcName, pvcUid: pvc.metadata.uid };
};
const stopWorker = identity => kube('delete', 'pod', identity.name, '--wait=true', '--timeout=30s');
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
  await gateway('UpdateAgentConfiguration', { idempotencyKey: key(), expectedRevisionId: configuration.revisionId, configuration: { workspaceId: workspace.id, provider: 'openai', model: 'fixture-model', instructions: 'Use only this frozen conversation context.', maxInputTokens: 65536, maxOutputTokens: 4096, maxTotalTokens: 131072, idleTimeoutSeconds: configuration.idleTimeoutSeconds } });
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
  const tools = await send('fixture:tools');
  await terminal(tools.turnId);
  const toolCall = (await calls()).find(call => call.turnId === tools.turnId);
  assert.equal(toolCall.state, 'completed', 'all three bounded reads complete before the final response');
  assert.deepEqual(toolCall.toolInvocations.map(tool => tool.name).sort(), ['read_workspace_instructions', 'read_project_status', 'read_conversation_history'].sort());
  assert.ok(toolCall.toolInvocations.every(tool => tool.invocationId && tool.inputDigest && tool.outputDigest));
  assert.equal(toolCall.totalTokens, '40', 'tool continuation usage accumulates across both requests');
  const afterTools = await stats();
  assert.equal(afterTools.calls, afterReload.calls + 2, 'one tool request and one continuation');
  assert.equal(afterTools.toolContinuations, (before.toolContinuations || 0) + 1);
  await page.reload();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  assert.equal((await stats()).calls, afterTools.calls, 'tool result replay does not call the provider');
  const completedSession = (await gateway('ListConversationSessions', scope)).sessions[0];
  const completedWorker = workerIdentity(completedSession);
  stopWorker(completedWorker);
  const second = await send('fixture:no-usage');
  await terminal(second.turnId);
  const adoptedSession = (await gateway('ListConversationSessions', scope)).sessions[0];
  assert.equal(adoptedSession.id, completedSession.id);
  assert.equal(adoptedSession.generation, completedSession.generation);
  const adoptedWorker = workerIdentity(adoptedSession);
  assert.notEqual(adoptedWorker.podUid, completedWorker.podUid, 'the completed worker was recreated');
  assert.equal(adoptedWorker.pvcUid, completedWorker.pvcUid, 'adoption keeps the accepted-result ledger');
  assert.equal((await stats()).calls, afterTools.calls + 1, 'adoption does not resend the prior completed tool turn');
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
  assert.equal((retained.recentMessages || []).filter(message => message.role === 'MESSAGE_ROLE_ASSISTANT').length, 4);
  await page.locator('#refresh-provider-calls').click();
  const acknowledgeButton = page.locator(`[data-acknowledge-call="${cancelled.callId}"]`);
  await acknowledgeButton.waitFor();
  assert.equal(await acknowledgeButton.isDisabled(), true, 'possible-charge consent is required');
  await page.locator(`[data-acknowledge-consent="${cancelled.callId}"]`).check();
  let acknowledgementCommand;
  await page.route('**/api/AcknowledgeProviderUncertainty', async route => {
    acknowledgementCommand = route.request().postDataJSON();
    const accepted = await route.fetch();
    assert.equal(accepted.status(), 200);
    await route.abort('connectionfailed');
  }, { times: 1 });
  await acknowledgeButton.click();
  await waitFor(() => page.evaluate(() => JSON.parse(sessionStorage.getItem('pending'))), value => value?.method === 'AcknowledgeProviderUncertainty', 'retained uncertainty acknowledgement');
  await page.reload();
  await page.locator('#pending').waitFor({ state: 'visible' });
  const acknowledgementReplay = page.waitForResponse(response => new URL(response.url()).pathname === '/api/AcknowledgeProviderUncertainty');
  await page.locator('#retry').click();
  const acknowledgedResponse = await acknowledgementReplay;
  assert.equal(acknowledgedResponse.status(), 200);
  assert.deepEqual(acknowledgedResponse.request().postDataJSON(), acknowledgementCommand);
  assert.equal(acknowledgementCommand.acknowledgePossibleCharge, true);
  await page.locator('#pending').waitFor({ state: 'hidden' });
  const acknowledged = (await calls()).find(call => call.turnId === uncertainTurn.turnId);
  assert.equal(acknowledged.state, 'uncertain', 'acknowledgement preserves the factual external outcome');
  assert.equal(acknowledged.uncertaintyAcknowledged, true);
  assert.equal((await stats()).calls, uncertainCalls, 'acknowledgement never retries a possibly paid call');
  await page.getByText(/Uncertain outcome acknowledged/).waitFor();
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
  for (const mode of ['tool-scope', 'tool-limit', 'tool-args', 'tool-unknown']) {
    const rejectedConversation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: `Bounded ${mode}`, idempotencyKey: key() })).conversation;
    scope = { ...scope, conversationId: rejectedConversation.id };
    const count = (await stats()).calls;
    const rejected = await send(`fixture:${mode}`);
    const value = await terminal(rejected.turnId);
    const rejectedTurn = value.recentTerminalTurns.find(turn => turn.id === rejected.turnId);
    assert.equal(rejectedTurn.presentationState, 'PRESENTATION_STATE_FAILED', `${mode} fails explicitly`);
    assert.match(rejectedTurn.safeFailureReason, /^provider_tool_/, `${mode} exposes a safe tool rejection`);
    assert.equal((await stats()).calls, count + 1, `${mode} never sends a continuation`);
    assert.equal(value.recentMessages.filter(message => message.role === 'MESSAGE_ROLE_ASSISTANT').length, 0);
  }
  const restartConversation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: 'Continuation sending restart', idempotencyKey: key() })).conversation;
  scope = { ...scope, conversationId: restartConversation.id };
  const beforeRestart = await stats();
  const restartTurn = await send('fixture:tool-restart');
  await waitFor(stats, value => (value.modes['tool-restart'] || 0) === (beforeRestart.modes['tool-restart'] || 0) + 2, 'continuation accepted before its delayed result');
  const sendingSession = (await gateway('ListConversationSessions', scope)).sessions[0];
  const sendingWorker = workerIdentity(sendingSession);
  stopWorker(sendingWorker);
  // Duplicate admission marks the call uncertain before its signed recovery report
  // carries the retained first response and tool evidence. Wait for that report's
  // response identity before assessing the recovered invocation metadata.
  const recoveredCalls = await waitFor(calls, values => values.some(call => call.turnId === restartTurn.turnId && call.state === 'uncertain' && call.responseId), 'signed sending-continuation recovery evidence');
  const recovered = recoveredCalls.find(call => call.turnId === restartTurn.turnId);
  assert.equal(recovered.totalTokens, undefined, 'the second external outcome keeps cumulative usage unknown');
  assert.equal(recovered.toolInvocations.length, 1, 'the first retained tool result survives worker loss');
  const recoveredSession = (await gateway('ListConversationSessions', scope)).sessions[0];
  assert.equal(recoveredSession.id, sendingSession.id);
  assert.equal(recoveredSession.generation, sendingSession.generation);
  const recoveredWorker = workerIdentity(recoveredSession);
  assert.notEqual(recoveredWorker.podUid, sendingWorker.podUid);
  assert.equal(recoveredWorker.pvcUid, sendingWorker.pvcUid, 'interruption preserves the unresolved ledger');
  const requestCount = (await stats()).calls;
  assert.equal(requestCount, beforeRestart.calls + 2);
  await page.evaluate(scope => sessionStorage.setItem('selection', JSON.stringify(scope)), scope);
  await page.reload();
  await page.getByText(/The external outcome is unknown/).waitFor();
  assert.equal((await stats()).calls, requestCount, 'recovered sending intent never resends the continuation');
  assert.equal((await snapshot()).recentMessages.filter(message => message.role === 'MESSAGE_ROLE_ASSISTANT').length, 0);
  // M3 proposals extend this same provider/session acceptance topology.
  const proposalConversation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: 'Agent proposal review', idempotencyKey: key() })).conversation;
  scope = { ...scope, conversationId: proposalConversation.id };
  const beforeProposal = (await stats()).calls;
  const proposed = await send('fixture:proposal');
  const proposedSnapshot = await terminal(proposed.turnId);
  assert.equal(proposedSnapshot.plans.length, 1);
  const plan = proposedSnapshot.plans[0];
  const version = (await gateway('GetPlanVersion', { ...scope, planId: plan.id })).version;
  assert.equal(version.author, 'agent');
  assert.equal(version.provenance.sourceTurnId, proposed.turnId);
  assert.equal(version.provenance.providerResponseId, (await calls()).find(value => value.turnId === proposed.turnId).responseId);
  assert.ok(version.provenance.assumptions.length && version.provenance.evidence.length);
  assert.equal((await stats()).calls, beforeProposal + 1, 'proposal is a terminal structured result with no continuation');
  await page.evaluate(scope => sessionStorage.setItem('selection', JSON.stringify(scope)), scope);
  await page.reload();
  await page.locator(`[data-plan-id="${plan.id}"]`).click();
  await page.getByText('Declared evidence · unverified', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.proposalInjected), undefined);
  assert.match(await page.locator('#plan-review').innerText(), /The user wants a reviewable proposal/);
  assert.equal((await gateway('GetPlanVersion', { ...scope, planId: plan.id })).version.id, version.id);
  assert.equal((await stats()).calls, beforeProposal + 1, 'reconnect preserves one accepted version without another call');
  // A provider proposal becomes admissible only after operator review resolves scope.
  const proposalTasks = await gateway('ListTasks', scope);
  assert.equal((proposalTasks.tasks || []).length, 0, 'model proposal grants no task authority');
  const resolvedContent = structuredClone(version.content);
  resolvedContent.unresolvedQuestions = [];
  const reviewed = (await gateway('RevisePlan', { ...scope, planId: plan.id, expectedVersionId: version.id, content: resolvedContent, idempotencyKey: key() })).version;
  await page.locator(`[data-plan-id="${plan.id}"]`).click();
  await page.waitForFunction(id => document.querySelector('#plan-approval-basis')?.textContent.includes(id), reviewed.id);
  await page.locator('#plan-approve-consent').check();
  const approving = page.waitForResponse(response => response.url().endsWith('/api/ApprovePlan'));
  await page.locator('#plan-approve').click();
  const approvalResponse = await approving;
  assert.equal(approvalResponse.status(), 200);
  const admitted = await approvalResponse.json();
  assert.equal(admitted.approval.versionId, reviewed.id);
  assert.equal(admitted.approval.contentDigest, reviewed.contentDigest);
  assert.equal(admitted.tasks.length, version.content.tasks.length);
  for (const [index, task] of admitted.tasks.entries()) {
    assert.equal(task.conversationId, proposalConversation.id);
    assert.equal(task.projectId, project.id);
    assert.equal(task.versionId, reviewed.id);
    assert.equal(task.state, 'awaiting_execution');
    assert.deepEqual(task.specification, version.content.tasks[index]);
  }
  await page.reload();
  assert.deepEqual(await gateway('GetPlanApproval', { ...scope, planId: plan.id }), admitted);
  assert.deepEqual((await gateway('GetPlanVersion', { ...scope, planId: plan.id, versionId: version.id })).version, version, 'approval preserves original agent provenance');
  assert.equal((await stats()).calls, beforeProposal + 1, 'review and approval create no provider calls');
  const foreignStatus = await page.evaluate(async body => (await fetch('/api/GetPlanVersion', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body) })).status, { ...uncertainScope, planId: plan.id });
  assert.equal(foreignStatus, 404, 'another conversation cannot read the proposal');
  for (const mode of ['proposal-malformed', 'proposal-scope', 'proposal-mixed']) {
    const conversation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: `Bounded ${mode}`, idempotencyKey: key() })).conversation;
    scope = { ...scope, conversationId: conversation.id };
    const priorCalls = (await stats()).calls;
    const receipt = await send(`fixture:${mode}`);
    const value = await terminal(receipt.turnId);
    assert.equal(value.recentTerminalTurns.find(turn => turn.id === receipt.turnId).presentationState, 'PRESENTATION_STATE_FAILED');
    assert.equal((value.plans || []).length, 0, 'invalid proposals never become versions');
    assert.equal((await stats()).calls, priorCalls + 1, 'invalid proposals never trigger another request');
  }
  const proposalCancellation = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, title: 'Cancelled proposal', idempotencyKey: key() })).conversation;
  scope = { ...scope, conversationId: proposalCancellation.id };
  const cancelledProposal = await send('fixture:proposal-delay');
  await waitFor(calls, values => values.some(value => value.turnId === cancelledProposal.turnId && value.state === 'sending'), 'proposal sending boundary');
  await gateway('CancelTurn', { ...scope, turnId: cancelledProposal.turnId, idempotencyKey: key() });
  await waitFor(calls, values => values.some(value => value.turnId === cancelledProposal.turnId && value.cancellationRequested && ['uncertain', 'cancelled', 'completed'].includes(value.state)), 'cancelled proposal outcome');
  assert.equal(((await snapshot()).plans || []).length, 0, 'cancelled generation cannot write a proposal');
  scope = uncertainScope;
  assert.deepEqual(errors, []);
  if (process.env.CONSOLE_EVIDENCE_DIRECTORY) {
    await mkdir(process.env.CONSOLE_EVIDENCE_DIRECTORY, { recursive: true });
    await page.screenshot({ path: `${process.env.CONSOLE_EVIDENCE_DIRECTORY}/provider-console.png`, fullPage: true });
  }
  console.log(JSON.stringify({ outcome: 'passed', scope, checks: ['scripted-to-openai-cutover', 'responses-request-profile', 'retained-response-usage', 'reload-no-resend', 'unknown-usage-reservation', 'uncertainty-no-resend', 'queued-cancellation-no-call', 'in-flight-cancellation', 'cancellation-receipt-replay', 'exactly-once-visible-results', 'three-scoped-read-tools', 'tool-usage-accounting', 'tool-replay-no-resend', 'foreign-tool-scope-rejected', 'tool-invocation-limit', 'tool-argument-limit', 'unknown-tool-rejected', 'uncertainty-acknowledgement-consent', 'uncertainty-acknowledgement-receipt-replay', 'uncertainty-evidence-preserved', 'completed-tool-worker-adoption', 'continuation-sending-worker-loss', 'retained-pvc-identity', 'continuation-no-resend', 'agent-proposal-version', 'agent-proposal-provenance', 'operator-reviewed-proposal-admission', 'approved-proposal-origin-reload', 'approval-no-provider-call', 'proposal-review-reconnect', 'proposal-escaping', 'proposal-cross-conversation-read-rejection', 'malformed-proposal-rejection', 'proposal-scope-spoof-rejection', 'mixed-proposal-rejection', 'cancelled-proposal-no-version'], calls: (await stats()).calls - before.calls }));
} finally { await browser.close(); }
