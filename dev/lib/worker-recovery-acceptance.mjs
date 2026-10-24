// Actual coding workers and managed quality; the coding provider is a deterministic fixture.
import assert from 'node:assert/strict';
import { randomUUID, createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const root = process.cwd();
const evidence = process.argv[2];
const endpoint = process.env.CONSOLE_URL;
const fixture = process.env.PROVIDER_FIXTURE_URL;
const address = process.env.AYNI_TRUSTED_GRPC_ADDRESS;
const namespace = process.env.AYNI_ORCHESTRATOR_NAMESPACE;
assert(evidence && path.isAbsolute(evidence));
assert.match(namespace || '', /^ayni-orchestrator-acceptance-[a-z0-9-]+$/);
assert.match(address || '', /^127\.0\.0\.1:[0-9]+$/);
for (const value of [endpoint, fixture]) { const url = new URL(value); assert.equal(url.hostname, '127.0.0.1'); assert.equal(url.protocol, 'http:'); }
fs.mkdirSync(evidence);
const file = name => path.join(evidence, name);
const output = (name, value) => fs.writeFileSync(file(name), JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const runtime = process.env.AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE || JSON.parse(fs.readFileSync('.ayni/task-quality-runtime.json', 'utf8')).runtime_image;
assert.match(runtime, /@sha256:[a-f0-9]{64}$/);
let stage = 'initialize';
const report = { schema: 'build.ayni.orchestrator.worker-recovery-acceptance.v1', result: 'failed', gates: [], namespace, runtime_image: runtime, fixture_boundaries: { coding: 'deterministic OpenAI Responses protocol fixture', quality: 'actual managed Ayni runtime', live_provider: false, live_github: false } };
async function rpc(method, body, name, expected = 200) {
  const response = await fetch(`${endpoint}/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'x-console-request': '1', Origin: endpoint }, body: JSON.stringify(body), signal: AbortSignal.timeout(30_000) });
  const value = await response.json();
  if (name) output(name, { request: body, response: value, status: response.status });
  assert.equal(response.status, expected, `${method}: ${JSON.stringify(value)}`); return value;
}
const stats = async () => { const response = await fetch(`${fixture}/stats`, { signal: AbortSignal.timeout(5000) }); assert(response.ok); return response.json(); };
const trusted = (mode, name, request) => {
  output(`${name}-request.json`, request);
  const value = execFileSync(path.join(root, 'target/debug/workflow-prepare-task'), [mode, address, file(`${name}-request.json`)], { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
  const parsed = JSON.parse(value); output(`${name}-receipt.json`, parsed); return parsed;
};
const kube = (...args) => execFileSync('kubectl', ['--context', 'docker-desktop', '--namespace', namespace, ...args], { encoding: 'utf8' });
async function waitFor(read, check, label) {
  for (let n = 0; n < 1200; n++) { const value = await read(); if (check(value)) return value; await delay(500); }
  throw new Error(`${label} timed out`);
}
try {
  await waitFor(async () => { try { return (await fetch(endpoint)).ok; } catch { return false; } }, value => value, 'owned gateway readiness');
  assert.equal((await stats()).calls, 0, 'fresh coding fixture process required');
  const metadata = JSON.parse(fs.readFileSync('dev/fixtures/task-quality/source.json', 'utf8'));
  const pack = fs.readFileSync('dev/fixtures/task-quality/source.pack');
  assert.equal(`sha256:${createHash('sha256').update(pack).digest('hex')}`, metadata.source_pack_digest);
  const workspace = (await rpc('CreateWorkspace', { idempotencyKey: randomUUID(), displayName: 'Worker-originated recovery fixture' }, 'workspace.json')).workspace;
  execFileSync('kubectl', ['--context', 'docker-desktop', '--namespace', `${namespace}-sessions`, 'apply', '-f', '-'], { input: JSON.stringify({ apiVersion: 'v1', kind: 'Secret', metadata: { name: `provider-openai-${workspace.id.replaceAll('-', '')}` }, type: 'Opaque', stringData: { 'api-key': 'provider-fixture-key' } }), stdio: ['pipe', 'ignore', 'pipe'] });
  const configuration = (await rpc('GetAgentConfiguration', { workspaceId: workspace.id })).configuration;
  await rpc('UpdateAgentConfiguration', { idempotencyKey: randomUUID(), expectedRevisionId: configuration.revisionId, configuration: { workspaceId: workspace.id, provider: 'openai', model: 'fixture-model', instructions: 'Only use the isolated coding protocol fixture. Questions and proposals require explicit operator review.', maxInputTokens: 65536, maxOutputTokens: 4096, maxTotalTokens: 131072, idleTimeoutSeconds: 300 } }, 'configuration.json');
  const project = (await rpc('CreateProject', { workspaceId: workspace.id, idempotencyKey: randomUUID(), displayName: 'Bounded worker recovery', goal: 'Ask, review and confirm without automatic continuation.' })).project;
  const conversation = (await rpc('CreateConversation', { workspaceId: workspace.id, projectId: project.id, idempotencyKey: randomUUID(), title: 'Worker question origin' })).conversation;
  const scope = { workspaceId: workspace.id, projectId: project.id, conversationId: conversation.id }; report.scope = scope;
  const version = (await rpc('CreatePlan', { ...scope, idempotencyKey: randomUUID(), content: { title: 'Explicit worker question', objective: 'Retain a model question and authorize one answered continuation.', scope: 'Only src/lib.rs in the managed task fixture.', tasks: [{ key: 'question', objective: 'Ask before appending the fixture comment.', scope: 'Retain the exact original test; append only the reviewed comment.', acceptanceCriteria: ['One worker question and model proposal.', 'No execution without exact confirmation.', 'Actual managed quality passes after the answered continuation.'], repository: 'https://example.invalid/task-source.git', sourceRef: 'refs/heads/main', sourcePaths: ['src/lib.rs'] }] } }, 'plan.json')).version;
  const approval = await rpc('ApprovePlan', { ...scope, idempotencyKey: randomUUID(), planId: version.planId, expectedVersionId: version.id, contentDigest: version.contentDigest }, 'approval.json');
  assert.equal(approval.tasks.length, 1); const task = approval.tasks[0];
  const preparation = trusted('submit', 'preparation', { ...scope, taskId: task.id, idempotencyKey: randomUUID(), inputs: { repository: task.specification.repository, sourceRef: task.specification.sourceRef, sourceCommit: metadata.source_commit, lockDigest: metadata.lock_digest, runtimeImage: runtime, maxSeconds: 300, maxTokens: 10000 } }).preparation;
  assert(preparation.attemptId && !preparation.blockers?.length);
  const selection = { ...scope, taskId: task.id, attemptId: preparation.attemptId };
  trusted('submit-start', 'start', { ...selection, idempotencyKey: randomUUID(), sourcePack: pack.toString('base64') });
  stage = 'worker-question';
  const parent = await waitFor(() => rpc('GetTaskAttempt', selection), value => value.attempt?.state === 'failed' && value.attempt.safeReason === 'worker_question', 'worker-originated question'); output('parent.json', parent);
  const history = await rpc('ListTaskRecovery', { ...selection, limit: 32 }, 'worker-records.json');
  assert.equal(history.records.length, 2); assert(history.records.every(value => value.author === 'agent'));
  const question = history.records.find(value => value.kind === 'question'); const proposal = history.records.find(value => value.kind === 'proposal');
  assert(question && proposal); assert.equal(proposal.proposal.action, 'continue'); assert.equal(proposal.proposal.additionalSeconds, 300); assert.equal(proposal.proposal.additionalTokens, 10000);
  report.gates.push('worker-origin-question-and-proposal');
  const confirmationRequest = { ...scope, taskId: task.id, proposalId: proposal.id, proposalDigest: proposal.contentDigest, idempotencyKey: randomUUID(), acknowledgeProviderUncertainty: false };
  await rpc('ConfirmTaskRecovery', confirmationRequest, 'unanswered-confirmation.json', 409);
  assert.equal((await stats()).calls, 1); report.gates.push('unanswered-question-blocks-confirmation');
  stage = 'worker-question-restart';
  output('question-supervisor-restart.json', { output: kube('rollout', 'restart', 'deployment/task-supervisor') + kube('rollout', 'status', 'deployment/task-supervisor', '--timeout=3m') });
  assert.deepEqual(await rpc('ListTaskRecovery', { ...selection, limit: 32 }), history); assert.equal((await stats()).calls, 1);
  const answerRequest = { ...selection, idempotencyKey: randomUUID(), answer: { questionId: question.id, text: 'Retain the original test and append only the reviewed fixture comment.' } };
  const answer = await rpc('RecordTaskRecovery', answerRequest, 'answer.json'); assert.deepEqual(await rpc('RecordTaskRecovery', answerRequest), answer);
  const confirmation = await rpc('ConfirmTaskRecovery', confirmationRequest, 'confirmation.json'); assert.deepEqual(await rpc('ConfirmTaskRecovery', confirmationRequest), confirmation);
  report.gates.push('answer-and-exact-confirmation');
  stage = 'answered-managed-quality';
  const childSelection = { ...scope, taskId: task.id, attemptId: confirmation.confirmation.attemptId };
  const child = await waitFor(() => rpc('GetTaskAttempt', childSelection), value => value.attempt?.state === 'completed' && value.attempt.quality?.state === 'passed', 'answered continuation with actual managed quality');
  assert.equal(child.attempt.sourceCommit, metadata.source_commit); assert.equal(child.attempt.parentAttemptId, selection.attemptId); assert.equal(child.attempt.recoveryProposalId, proposal.id);
  assert.equal(child.attempt.maxSeconds, 300); assert.equal(child.attempt.maxTokens, 10000);
  assert.match(child.attempt.patch, /Continued after explicit review of the worker question/); assert.equal(child.attempt.quality.completedTargets, child.attempt.quality.expectedTargets); output('child.json', child);
  const counters = await stats(); assert.equal(counters.calls, 2); assert.equal(counters.questions, 1); assert.equal(counters.continuations, 1); assert.deepEqual(counters.answeredQuestionIds, [question.id]); assert.equal(new Set(counters.requestIds).size, 2);
  output('provider-counters.json', counters); report.gates.push('answered-model-recovery-managed-quality');
  const answeredHistory = await rpc('ListTaskRecovery', { ...selection, limit: 32 }, 'answered-records.json');
  stage = 'accepted-replay';
  output('completed-supervisor-restart.json', { output: kube('rollout', 'restart', 'deployment/task-supervisor') + kube('rollout', 'status', 'deployment/task-supervisor', '--timeout=3m') });
  assert.deepEqual(await rpc('ConfirmTaskRecovery', confirmationRequest, 'replayed-confirmation.json'), confirmation);
  assert.deepEqual(await rpc('ListTaskRecovery', { ...selection, limit: 32 }, 'replayed-records.json'), answeredHistory);
  assert.deepEqual(await rpc('GetTaskAttempt', childSelection, 'replayed-child.json'), child); assert.deepEqual(await rpc('GetTaskAttempt', selection, 'replayed-parent.json'), parent);
  assert.deepEqual(await stats(), counters); report.gates.push('worker-recovery-restart-no-resend');
  report.task_id = task.id; report.parent_attempt_id = selection.attemptId; report.child_attempt_id = childSelection.attemptId; report.result = 'passed'; stage = 'complete';
} catch (error) { report.error = error.message; throw error; }
finally {
  report.stage = stage; report.cleanup = 'retained_namespace_and_all_authoritative_stores';
  report.artifacts_sha256 = Object.fromEntries(fs.readdirSync(evidence).filter(name => name !== 'outcome.json').map(name => [name, createHash('sha256').update(fs.readFileSync(file(name))).digest('hex')]));
  output('outcome.json', report);
}
