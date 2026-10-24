// Deployed multi-upstream integration and confirmed repair using explicit coding fixtures.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';

const [root, evidence, namespace, runtime, fixtureImage] = process.argv.slice(2);
assert(root && evidence && /^ayni-orchestrator-acceptance-/.test(namespace));
assert.match(runtime, /@sha256:[a-f0-9]{64}$/);
const repository = 'https://github.com/fixture/repo.git';
const file = name => path.join(evidence, name);
const output = (name, value) => fs.writeFileSync(file(name), typeof value === 'string' ? value : JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const digest = bytes => `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const children = [];
let stage = 'initialize';
const report = { schema: 'build.ayni.orchestrator.dependency-acceptance.v1', result: 'failed', gates: [], fixture_boundaries: { coding: 'explicit deterministic fixture', quality: 'actual managed Ayni runtime', publication: 'real Git transport and GitHub protocol fixture', live_provider: false, live_github: false }, namespace, runtime_image: runtime };
function command(program, args, filename, options = {}) {
  try {
    const result = execFileSync(program, args, { cwd: root, encoding: 'utf8', maxBuffer: 24 * 1024 * 1024, ...options });
    if (filename) output(filename, result);
    return result.trim();
  } catch (error) {
    if (filename) output(filename, `${error.stdout || ''}\n${error.stderr || ''}`);
    throw new Error(`${program} failed${filename ? `; see ${filename}` : ''}`);
  }
}
const kube = (...args) => command('kubectl', ['--context', 'docker-desktop', '--namespace', namespace, ...args]);
async function port() {
  const listener = net.createServer();
  await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
  const { port } = listener.address();
  await new Promise(resolve => listener.close(resolve));
  return port;
}
function launch(program, args, filename) {
  const log = fs.openSync(file(filename), 'wx');
  const child = spawn(program, args, { cwd: root, stdio: ['ignore', log, log] });
  fs.closeSync(log); children.push(child); return child;
}
async function forward(target, ports, filename) {
  const child = launch('kubectl', ['--context', 'docker-desktop', '--namespace', namespace, 'port-forward', '--address', '127.0.0.1', target, ...ports], filename);
  for (let n = 0; n < 100; n++) {
    assert.equal(child.exitCode, null, 'owned port-forward exited');
    if (fs.readFileSync(file(filename), 'utf8').includes('Forwarding from')) return child;
    await delay(100);
  }
  throw new Error('owned port-forward did not start');
}
const grpcPort = await port(); const consolePort = await port(); const gitPort = await port(); const fixturePort = await port();
const address = `127.0.0.1:${grpcPort}`; const base = `http://127.0.0.1:${consolePort}`;
let generation = 0; let gateway; let grpc;
async function connect() {
  for (const child of [gateway, grpc]) if (child) child.kill();
  await delay(300);
  const pods = JSON.parse(kube('get', 'pods', '-l', 'app.kubernetes.io/name=ayni-orchestrator-control-plane', '--sort-by=.metadata.creationTimestamp', '-o', 'json'));
  const pod = pods.items.filter(p => !p.metadata.deletionTimestamp && p.status?.conditions?.some(c => c.type === 'Ready' && c.status === 'True')).at(-1);
  assert(pod, 'ready control plane missing'); generation++;
  output(`control-plane-${generation}.json`, { name: pod.metadata.name, uid: pod.metadata.uid });
  grpc = await forward(`pod/${pod.metadata.name}`, [`${grpcPort}:50051`], `grpc-${generation}.txt`);
  gateway = launch(path.join(root, 'target/debug/workflow-console'), [`127.0.0.1:${consolePort}`, address], `console-${generation}.txt`);
  for (let n = 0; n < 100; n++) {
    try { if ((await fetch(base, { signal: AbortSignal.timeout(1000) })).ok) return; } catch {}
    await delay(100);
  }
  throw new Error('owned gateway did not start');
}
async function rpc(method, body, filename, expected = 200) {
  const response = await fetch(`${base}/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'x-console-request': '1', Origin: base }, body: JSON.stringify(body), signal: AbortSignal.timeout(30_000) });
  const result = await response.json();
  if (filename) output(filename, { request: body, response: result, status: response.status });
  assert.equal(response.status, expected, `${method}: ${JSON.stringify(result)}`);
  return result;
}
const trusted = (binary, args, filename) => JSON.parse(command(path.join(root, 'target/debug', binary), args, filename));
async function waitAttempt(selection, label, predicate) {
  const end = Date.now() + 600_000; let last;
  while (Date.now() < end) {
    last = (await rpc('GetTaskAttempt', selection)).attempt;
    if (predicate(last)) { output(`${label}.json`, last); return last; }
    if (last && ['failed', 'uncertain', 'cancelled', 'interrupted'].includes(last.state)) {
      output(`${label}-failure.json`, last); throw new Error(`${label} ended ${last.state}: ${last.safeReason || ''}`);
    }
    await delay(500);
  }
  output(`${label}-timeout.json`, last || {}); throw new Error(`${label} timed out`);
}
const checkout = file('trusted-source');
const git = (...args) => command('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', '-C', checkout, ...args]);
try {
  await connect();
  const workspace = (await rpc('CreateWorkspace', { idempotencyKey: randomUUID(), displayName: 'Deployed dependency acceptance' }, 'workspace.json')).workspace;
  const project = (await rpc('CreateProject', { idempotencyKey: randomUUID(), workspaceId: workspace.id, displayName: 'Multi-upstream fixture', goal: 'Freeze accepted upstream drafts and retain them through dependent repair.' }, 'project.json')).project;
  const conversation = (await rpc('CreateConversation', { idempotencyKey: randomUUID(), workspaceId: workspace.id, projectId: project.id, title: 'Reviewed integrated task inputs' }, 'conversation.json')).conversation;
  const scope = { workspaceId: workspace.id, projectId: project.id, conversationId: conversation.id }; report.scope = scope;
  const taskSpecs = ['first', 'second', 'dependent'].map((key, index) => ({ key, objective: index === 2 ? 'Task quality failure fixture' : `Task ${key} upstream fixture`, scope: 'Only the isolated managed task fixture.', acceptanceCriteria: ['Retain accepted coding, managed quality and exact source identities.'], repository, sourceRef: 'refs/heads/main', dependsOn: index === 2 ? ['first', 'second'] : [] }));
  const version = (await rpc('CreatePlan', { ...scope, idempotencyKey: randomUUID(), content: { title: 'Two accepted upstream drafts', objective: 'Integrate two explicit draft commits before dependent execution.', scope: 'Explicit deterministic local fixtures only.', tasks: taskSpecs } }, 'plan.json')).version;
  const approved = await rpc('ApprovePlan', { ...scope, idempotencyKey: randomUUID(), planId: version.planId, expectedVersionId: version.id, contentDigest: version.contentDigest }, 'approval.json');
  assert.equal(approved.tasks.length, 3);
  const metadata = JSON.parse(fs.readFileSync(path.join(root, 'dev/fixtures/task-quality/source.json'), 'utf8'));
  const pack = fs.readFileSync(path.join(root, 'dev/fixtures/task-quality/source.pack'));
  assert.equal(digest(pack), metadata.source_pack_digest);
  fs.mkdirSync(checkout); git('init', '--initial-branch=main');
  command('git', ['-C', checkout, 'unpack-objects', '-q'], 'source-unpack.txt', { input: pack });
  git('update-ref', 'refs/heads/main', metadata.source_commit); git('checkout', 'main'); git('remote', 'add', 'origin', repository);
  report.source_commit = metadata.source_commit; report.lock_digest = metadata.lock_digest;
  stage = 'publication-binding';
  command('helm', ['upgrade', '--install', 'ayni-orchestrator', path.join(root, 'dev/charts/ayni-orchestrator-local'), '--reset-then-reuse-values', '--kube-context', 'docker-desktop', '--namespace', namespace, '--set', 'ayni-orchestrator.taskExecution.fixtureEnabled=true', '--set', 'publicationFixture.enabled=true', '--set-string', `publicationFixture.image=${fixtureImage}`, '--set', 'ayni-orchestrator.publication.enabled=true', '--set-string', `ayni-orchestrator.publication.workspaceId=${workspace.id}`, '--set-string', `ayni-orchestrator.publication.repository=${repository}`, '--set-string', 'ayni-orchestrator.publication.githubSecretName=publication-fixture-github', '--set', 'ayni-orchestrator.publication.fixtureEnabled=true', '--set-string', `ayni-orchestrator.publication.fixtureApiUrl=http://publication-fixture.${namespace}.svc:8080`, '--set-string', `ayni-orchestrator.publication.fixtureGitUrl=git://publication-fixture.${namespace}.svc:9418/fixture/repo.git`, '--wait', '--timeout=5m'], 'publication-binding.txt', { env: { ...process.env, HELM_NO_PLUGINS: '1' } });
  await connect();
  await forward('service/publication-fixture', [`${gitPort}:9418`, `${fixturePort}:8080`], 'publication-forward.txt');
  const remoteBefore = await (await fetch(`http://127.0.0.1:${fixturePort}/fixture/state`)).json(); output('remote-before.json', remoteBefore);
  const inputs = sourceCommit => ({ repository, sourceRef: 'refs/heads/main', sourceCommit, runtimeImage: runtime, maxSeconds: 300, maxTokens: 10000 });
  async function prepare(task, label, selectedInputs) {
    const request = { ...scope, taskId: task.id, idempotencyKey: randomUUID(), inputs: selectedInputs }; output(`${label}-selection.json`, request);
    command(path.join(root, 'target/debug/workflow-prepare-task'), ['verify', address, checkout, file(`${label}-selection.json`), file(`${label}-request.json`)], `${label}-verify.txt`);
    const preparation = trusted('workflow-prepare-task', ['submit', address, file(`${label}-request.json`)], `${label}-preparation.json`).preparation;
    assert(preparation.attemptId && !preparation.blockers?.length);
    assert.deepEqual(preparation.inputs, JSON.parse(fs.readFileSync(file(`${label}-request.json`), 'utf8')).inputs);
    command(path.join(root, 'target/debug/workflow-prepare-task'), ['start', address, checkout, file(`${label}-request.json`), file(`${label}-start.json`)], `${label}-start-verify.txt`);
    trusted('workflow-prepare-task', ['submit-start', address, file(`${label}-start.json`)], `${label}-receipt.json`);
    return { selection: { ...scope, taskId: task.id, attemptId: preparation.attemptId }, preparation };
  }
  async function publish(selection, label) {
    output(`${label}-publication-selection.json`, selection);
    command(path.join(root, 'target/debug/workflow-publish-task'), ['prepare', address, file(`${label}-publication-selection.json`), file(`${label}-publication-request.json`)], `${label}-publication-review.txt`);
    trusted('workflow-publish-task', ['submit', address, file(`${label}-publication-request.json`)], `${label}-publication-receipt.json`);
    return waitAttempt(selection, `${label}-draft`, a => a?.publication?.state === 'draft');
  }
  stage = 'upstream-managed-quality-and-publication';
  const upstreams = [];
  for (const [index, task] of approved.tasks.slice(0, 2).entries()) {
    const label = `upstream-${index}`; const prepared = await prepare(task, label, inputs(metadata.source_commit));
    const passed = await waitAttempt(prepared.selection, `${label}-quality`, a => a?.state === 'completed' && a.executionOutcome === 'completed' && a.quality?.state === 'passed');
    assert.equal(passed.quality.completedTargets, passed.quality.expectedTargets);
    const draft = await publish(prepared.selection, label);
    assert.equal(draft.publication.patchDigest, passed.patchDigest); assert.equal(draft.publication.qualityDigest, passed.qualityDigest);
    upstreams.push({ taskId: task.id, attemptId: prepared.selection.attemptId, commitSha: draft.publication.commitSha });
    git('fetch', `git://127.0.0.1:${gitPort}/fixture/repo.git`, `refs/heads/${draft.publication.branch}`);
  }
  assert.equal(new Set(upstreams.map(value => value.commitSha)).size, 2, 'upstream drafts must have distinct accepted commit identities');
  report.gates.push('two-accepted-upstream-draft-commits'); report.upstream_selections = upstreams;
  stage = 'trusted-source-integration';
  git('-c', 'user.name=Dependency Acceptance Fixture', '-c', 'user.email=fixture@example.invalid', 'merge', '--no-ff', '-m', 'test: integrate two reviewed upstream draft commits', ...upstreams.map(value => value.commitSha));
  const integrated = git('rev-parse', 'HEAD');
  for (const upstream of upstreams) git('merge-base', '--is-ancestor', upstream.commitSha, integrated);
  output('integrated-source.txt', git('log', '-4', '--format=%H %P'));
  report.integrated_source_commit = integrated; report.gates.push('trusted-integrated-source-ancestry');
  const dependent = approved.tasks[2];
  const prepared = await prepare(dependent, 'dependent', { ...inputs(integrated), upstreamSelections: upstreams });
  assert.deepEqual(prepared.preparation.inputs.upstreamSelections, upstreams);
  assert.equal(prepared.preparation.inputs.sourceCommit, integrated);
  report.gates.push('frozen-dependent-inputs');
  stage = 'dependent-managed-quality-failure';
  const failed = await waitAttempt(prepared.selection, 'dependent-quality-failed', a => a?.state === 'completed' && a.executionOutcome === 'completed' && a.quality?.state === 'failed');
  assert.equal(failed.sourceCommit, integrated); assert.equal(failed.runtimeImage, runtime); assert(failed.patch.includes('fails_quality'));
  const feedback = await rpc('ListTaskFeedback', { ...scope, limit: 32 }, 'dependent-feedback.json');
  const notice = feedback.feedback.find(value => value.attemptId === prepared.selection.attemptId && value.safeReason === 'quality_failed'); assert(notice);
  report.gates.push('managed-dependent-quality-failure');
  const recorded = await rpc('RecordTaskRecovery', { ...prepared.selection, idempotencyKey: randomUUID(), proposal: { feedbackId: notice.id, explanation: 'Repair the failed fixture test while preserving the two frozen accepted upstream selections.', action: 'repair_quality', additionalSeconds: 300, additionalTokens: 4096 } }, 'repair-proposal.json');
  const request = { ...scope, taskId: dependent.id, idempotencyKey: randomUUID(), proposalId: recorded.record.id, proposalDigest: recorded.record.contentDigest, acknowledgeProviderUncertainty: false };
  const confirmation = await rpc('ConfirmTaskRecovery', request, 'repair-confirmation.json');
  assert.deepEqual(await rpc('ConfirmTaskRecovery', request), confirmation);
  const childSelection = { ...scope, taskId: dependent.id, attemptId: confirmation.confirmation.attemptId };
  const repaired = await waitAttempt(childSelection, 'dependent-repaired', a => a?.state === 'completed' && a.executionOutcome === 'completed' && a.quality?.state === 'passed');
  assert.equal(repaired.sourceCommit, integrated); assert.equal(repaired.runtimeImage, runtime); assert.equal(repaired.parentAttemptId, prepared.selection.attemptId);
  assert.equal(repaired.savedInputPatch, failed.patch); assert.equal(repaired.recoveryProposalId, recorded.record.id);
  assert(repaired.patch.includes('repaired_quality')); assert.equal(repaired.quality.completedTargets, repaired.quality.expectedTargets);
  const preparationBefore = await rpc('GetTaskExecution', { ...scope, taskId: dependent.id });
  assert.deepEqual(preparationBefore.preparation.inputs.upstreamSelections, upstreams);
  const childDraft = await publish(childSelection, 'dependent-repaired');
  const remoteAfter = await (await fetch(`http://127.0.0.1:${fixturePort}/fixture/state`)).json(); output('remote-after.json', remoteAfter);
  assert.equal(remoteAfter.create_calls - remoteBefore.create_calls, 3); assert.equal(remoteAfter.pulls.length - remoteBefore.pulls.length, 3);
  report.gates.push('confirmed-dependent-repair');
  report.dependent = { task_id: dependent.id, parent_attempt_id: prepared.selection.attemptId, child_attempt_id: childSelection.attemptId, patch_digest: repaired.patchDigest, quality_digest: repaired.qualityDigest, publication_commit: childDraft.publication.commitSha };
  stage = 'owner-restarts-and-exact-replay';
  for (const workload of ['deployment/control-plane', 'deployment/task-supervisor', 'statefulset/workflow-postgres']) {
    const name = workload.split('/')[1];
    output(`restart-${name}.txt`, kube('rollout', 'restart', workload) + '\n' + kube('rollout', 'status', workload, '--timeout=3m'));
    await connect();
    assert.deepEqual(await rpc('ConfirmTaskRecovery', request, `confirmation-replay-${name}.json`), confirmation);
    assert.deepEqual(await rpc('GetTaskExecution', { ...scope, taskId: dependent.id }, `preparation-replay-${name}.json`), preparationBefore);
    assert.deepEqual((await rpc('GetTaskAttempt', childSelection, `child-replay-${name}.json`)).attempt, childDraft);
    assert.deepEqual((await rpc('GetTaskAttempt', prepared.selection, `parent-replay-${name}.json`)).attempt, failed);
  }
  report.gates.push('dependency-recovery-owner-restart-exact-replay'); report.result = 'passed'; stage = 'complete';
} catch (error) {
  report.error = error.message; throw error;
} finally {
  await Promise.all(children.filter(child => child.exitCode === null && child.signalCode === null).map(child => new Promise(resolve => {
    child.once('close', resolve); child.kill(); setTimeout(resolve, 3000).unref();
  })));
  report.stage = stage; report.cleanup = 'retained_namespace_and_all_authoritative_stores';
  const manifest = {};
  function collect(directory) {
    for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
      const location = path.join(directory, item.name);
      if (item.isDirectory()) { if (item.name !== '.git' && location !== checkout) collect(location); }
      else if (item.isFile() && item.name !== 'outcome.json') manifest[path.relative(evidence, location)] = createHash('sha256').update(fs.readFileSync(location)).digest('hex');
    }
  }
  collect(evidence); report.artifacts_sha256 = manifest;
  output('outcome.json', report);
}
