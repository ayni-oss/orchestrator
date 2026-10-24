// Deployed Kubernetes task lifecycle + deterministic provider fixture; no live AI calls.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const endpoint = process.env.CONSOLE_URL || 'http://127.0.0.1:3000';
const fixture = process.env.PROVIDER_FIXTURE_URL || 'http://127.0.0.1:18080';
const namespace = process.env.AYNI_ORCHESTRATOR_NAMESPACE;
const evidence = process.argv[2] || process.env.CONSOLE_EVIDENCE_DIRECTORY;
assert.ok(evidence && path.isAbsolute(evidence), 'new absolute evidence directory required');
assert.match(namespace || '', /^ayni-orchestrator-acceptance-[a-z0-9-]+$/);
assert.ok(namespace.length <= 54);
for (const value of [endpoint, fixture]) {
  const url = new URL(value);
  assert.equal(url.hostname, '127.0.0.1');
  assert.equal(url.protocol, 'http:');
}
await mkdir(evidence);
const root = new URL('../../', import.meta.url);
const metadata = JSON.parse(await readFile(new URL('dev/fixtures/task-quality/source.json', root), 'utf8'));
const pack = await readFile(new URL('dev/fixtures/task-quality/source.pack', root));
assert.equal(`sha256:${createHash('sha256').update(pack).digest('hex')}`, metadata.source_pack_digest);
const runtime = process.env.AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE || JSON.parse(await readFile(new URL('.ayni/task-quality-runtime.json', root), 'utf8')).runtime_image;
assert.match(runtime, /@sha256:[a-f0-9]{64}$/);
const kube = (scope, ...args) => execFileSync('kubectl', ['--context', 'docker-desktop', '-n', scope, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
const key = () => crypto.randomUUID();
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } });
const page = await context.newPage();
const pageErrors = [];
page.on('pageerror', error => pageErrors.push(error.message));
const output = { schema: 'build.ayni.orchestrator.assessment-acceptance.v1', result: 'failed', liveProvider: false, coding: 'explicit scripted worker fixture', quality: 'actual managed Ayni runtime', gates: [], metadata, runtime };
let stage = 'initialize';
const checkpoint = async () => writeFile(path.join(evidence, 'outcome.json'), JSON.stringify({ ...output, stage }, null, 2));
const gateway = async (method, body) => {
  const response = await page.evaluate(async ({ method, body }) => {
    const response = await fetch(`/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  }, { method, body });
  assert.equal(response.status, 200, `${method}: ${JSON.stringify(response.body)}`);
  return response.body;
};
const waitFor = async (read, predicate, label, duration = 240000) => {
  const until = Date.now() + duration;
  while (Date.now() < until) {
    const value = await read();
    if (predicate(value)) return value;
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  throw new Error(`Timed out: ${label}`);
};
try {
  await page.goto(endpoint);
  const health = await fetch(`${fixture}/health`);
  assert.equal(health.status, 200, 'explicit forwarded provider fixture required');
  const workspace = (await gateway('CreateWorkspace', { idempotencyKey: key(), displayName: `Assessment acceptance ${Date.now()}` })).workspace;
  // This public fixture credential grants no capability outside the local fixture.
  execFileSync('kubectl', ['--context', 'docker-desktop', '-n', `${namespace}-sessions`, 'apply', '-f', '-'], {
    input: JSON.stringify({ apiVersion: 'v1', kind: 'Secret', metadata: { name: `provider-openai-${workspace.id.replaceAll('-', '')}` }, type: 'Opaque', stringData: { 'api-key': 'provider-fixture-key' } }),
    stdio: ['pipe', 'ignore', 'pipe'],
  });
  const project = (await gateway('CreateProject', { workspaceId: workspace.id, idempotencyKey: key(), displayName: 'Assessment fixture', goal: 'Assess accepted outcomes without granting execution.' })).project;
  const origin = (await gateway('CreateConversation', { workspaceId: workspace.id, projectId: project.id, idempotencyKey: key(), title: 'Assessment origin' })).conversation;
  const scope = { workspaceId: workspace.id, projectId: project.id, conversationId: origin.id };
  output.scope = scope;
  const calls = async () => (await gateway('ListProviderCalls', scope)).calls || [];
  const snapshot = async () => (await gateway('GetConversationSnapshot', scope)).snapshot;
  const status = async () => gateway('GetOutcomeAssessmentStatus', scope);
  const version = (await gateway('CreatePlan', { ...scope, idempotencyKey: key(), content: {
    title: 'Deployed assessment trigger', objective: 'Expose real accepted task outcomes.', scope: 'Only the isolated task fixture.',
    tasks: ['Task cancellation fixture', 'Produce the deterministic fixture file'].map((objective, index) => ({ key: `task${index}`, objective, scope: 'Only the fixture checkout.', acceptanceCriteria: ['Retain coding and managed-quality evidence.'], repository: 'https://example.invalid/task-source.git', sourceRef: 'refs/heads/main' })),
  } })).version;
  const approval = await gateway('ApprovePlan', { ...scope, idempotencyKey: key(), planId: version.planId, expectedVersionId: version.id, contentDigest: version.contentDigest });
  assert.equal(approval.tasks.length, 2);
  const tasks = [];
  for (const task of approval.tasks) {
    const preparation = (await gateway('PrepareTaskExecution', { ...scope, idempotencyKey: key(), taskId: task.id, inputs: {
      repository: task.specification.repository, sourceRef: task.specification.sourceRef, sourceCommit: metadata.source_commit, lockDigest: metadata.lock_digest,
      runtimeImage: runtime, maxSeconds: 300, maxTokens: 10000,
    } })).preparation;
    assert.deepEqual(preparation.blockers || [], []);
    assert.ok(preparation.attemptId);
    tasks.push({ taskId: task.id, attemptId: preparation.attemptId });
  }
  const configuration = (await gateway('GetAgentConfiguration', { workspaceId: workspace.id })).configuration;
  const configured = (await gateway('UpdateAgentConfiguration', { idempotencyKey: key(), expectedRevisionId: configuration.revisionId, configuration: {
    workspaceId: workspace.id, provider: 'openai', model: 'fixture-model', instructions: 'fixture:assessment-delay-first. Assess exact frozen evidence; proposals require review.',
    maxInputTokens: 65536, maxOutputTokens: 4096, maxTotalTokens: 131072, idleTimeoutSeconds: 300,
  } })).configuration;
  output.configurationRevisionId = configured.revisionId;
  await page.evaluate(scope => sessionStorage.setItem('selection', JSON.stringify(scope)), scope);
  await page.reload();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  await page.locator('details').filter({ has: page.locator('#assessment-authorization') }).locator('summary').click();
  await page.locator('#assessment-count').fill('3');
  await page.locator('#assessment-consent').check();
  let consent;
  await page.route('**/api/AuthorizeOutcomeAssessments', async route => {
    consent = { request: route.request().postDataJSON() };
    const accepted = await route.fetch();
    assert.equal(accepted.status(), 200);
    consent.response = await accepted.json();
    await route.abort('connectionfailed');
  }, { times: 1 });
  await page.locator('#assessment-authorization button').click();
  await waitFor(() => page.evaluate(() => JSON.parse(sessionStorage.getItem('pending'))), value => value?.method === 'AuthorizeOutcomeAssessments', 'retained assessment consent retry');
  await page.reload();
  await page.locator('#pending').waitFor({ state: 'visible' });
  const replayed = page.waitForResponse(response => response.url().endsWith('/api/AuthorizeOutcomeAssessments'));
  await page.locator('#retry').click();
  const replay = await replayed;
  assert.equal(replay.status(), 200);
  assert.deepEqual(replay.request().postDataJSON(), consent.request);
  assert.deepEqual(await replay.json(), consent.response);
  output.consent = consent;
  output.gates.push('console-explicit-bounded-consent', 'consent-response-loss-reload-exact-retry');
  const start = task => gateway('StartTaskExecution', { ...scope, ...task, idempotencyKey: key(), sourcePack: pack.toString('base64') });
  const attempt = task => gateway('GetTaskAttempt', { ...scope, ...task });
  stage = 'ordinary-progress'; await checkpoint();
  await start(tasks[0]);
  await waitFor(() => attempt(tasks[0]), value => value.attempt.state === 'claimed', 'real coding worker claimed');
  // Span a full minute sweep while the fixture's coding worker sleeps before reporting.
  const progressUntil = Date.now() + 65000;
  while (Date.now() < progressUntil) {
    assert.equal((await calls()).length, 0, 'task progress must not invoke a model');
    assert.equal(Number((await status()).admittedAssessments || 0), 0);
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  output.gates.push('deployed-progress-spans-maintenance-without-model');
  stage = 'accepted-outcome-trigger'; await checkpoint();
  const firstCall = await waitFor(calls, values => values.some(call => call.state === 'sending'), 'first outcome assessment sending');
  output.firstCall = firstCall[0];
  await start(tasks[1]);
  await waitFor(() => attempt(tasks[1]), value => value.attempt.executionOutcome === 'completed', 'second accepted task result inside delayed assessment', 45000);
  const stale = await waitFor(snapshot, value => value.recentTerminalTurns?.some(turn => turn.id === output.firstCall.turnId && turn.safeFailureReason === 'stale_outcome_assessment'), 'stale assessment rejected');
  assert.equal(stale.plans.filter(plan => plan.id !== version.planId).length, 0, 'stale assessment materializes no plan');
  output.gates.push('accepted-worker-outcome-triggers-assessment', 'changed-outcome-rejects-stale-assessment');
  stage = 'reviewable-proposal'; await checkpoint();
  for (const task of tasks) {
    await waitFor(() => attempt(task), value => value.attempt.state === 'completed' && value.attempt.quality?.state === 'passed', 'actual managed task quality', 240000);
  }
  const accepted = await waitFor(snapshot, value => value.plans?.some(plan => plan.id !== version.planId), 'retained assessment plan proposal');
  const proposals = [];
  for (const plan of accepted.plans.filter(plan => plan.id !== version.planId)) {
    const proposed = (await gateway('GetPlanVersion', { ...scope, planId: plan.id })).version;
    assert.equal(proposed.author, 'agent');
    assert.ok(proposed.provenance.sourceTurnId);
    proposals.push(proposed);
  }
  assert.equal((await gateway('ListTasks', scope)).tasks.length, 2, 'assessment proposals authorize no new tasks');
  const retainedCalls = await calls();
  assert.ok(retainedCalls.length >= 2 && retainedCalls.length <= 3);
  const retainedStatus = await status();
  assert.equal(Number(retainedStatus.admittedAssessments), retainedCalls.length);
  await gateway('AuthorizeOutcomeAssessments', { ...scope, idempotencyKey: key(), expectedConfigurationRevisionId: configured.revisionId, maxAssessments: 0 });
  output.gates.push('originating-reviewable-proposal-with-provider-provenance', 'proposal-grants-no-execution', 'bounded-assessment-count', 'explicit-revocation');
  stage = 'supervisor-restart-replay'; await checkpoint();
  kube(namespace, 'rollout', 'restart', 'statefulset/scripted-executor');
  kube(namespace, 'rollout', 'status', 'statefulset/scripted-executor', '--timeout=3m');
  await page.reload();
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  assert.deepEqual(await calls(), retainedCalls, 'supervisor restart must not repeat accepted provider requests');
  for (const proposed of proposals) assert.deepEqual((await gateway('GetPlanVersion', { ...scope, planId: proposed.planId, versionId: proposed.id })).version, proposed);
  assert.equal(Number((await status()).maxAssessments || 0), 0);
  output.proposals = proposals; output.calls = retainedCalls;
  output.attempts = await Promise.all(tasks.map(attempt));
  output.gates.push('conversation-supervisor-restart-no-resend', 'exact-proposal-replay', 'retained-revocation');
  assert.deepEqual(pageErrors, []);
  await page.screenshot({ path: path.join(evidence, 'assessment-console.png'), fullPage: true });
  output.result = 'passed'; stage = 'complete'; await checkpoint();
  console.log(JSON.stringify({ result: output.result, evidence, gates: output.gates, scope }));
} catch (error) {
  output.error = error.message;
  await checkpoint();
  throw error;
} finally { await browser.close(); }
