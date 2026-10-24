// Actual Orchestrator source and managed runtime; explicit coding/GitHub protocol fixtures.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';

const [root, evidence, namespace, runtime, grpcPort, consolePort, fixtureBase] = process.argv.slice(2);
assert(root && evidence && /^ayni-orchestrator-acceptance-/.test(namespace));
assert(/@sha256:[a-f0-9]{64}$/.test(runtime));
assert(/^local\/[a-z0-9._/-]+:[a-zA-Z0-9._-]+$/.test(fixtureBase));
const fixtureRepository = 'https://github.com/fixture/repo.git';
const base = `http://127.0.0.1:${consolePort}`;
const binary = name => path.join(root, 'target/debug', name);
const output = (name, value) => fs.writeFileSync(path.join(evidence, name), typeof value === 'string' ? value : JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const sha = bytes => `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
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
async function rpc(method, body, filename) {
  const response = await fetch(`${base}/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'x-console-request': '1', Origin: base }, body: JSON.stringify(body), signal: AbortSignal.timeout(20_000) });
  const result = await response.json();
  if (filename) output(filename, { request: body, response: result });
  assert(response.ok, `${method}: ${JSON.stringify(result)}`);
  return result;
}
async function waitConsole() {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(base, { signal: AbortSignal.timeout(1000) })).ok) return; } catch {}
    await delay(100);
  }
  throw new Error('local contributor gateway did not become ready');
}
async function waitAttempt(selection, stage, check, maxSeconds) {
  const end = Date.now() + maxSeconds * 1000;
  let last;
  while (Date.now() < end) {
    last = (await rpc('GetTaskAttempt', selection)).attempt;
    assert(last, 'attempt missing');
    if (check(last)) { output(`${stage}.json`, last); return last; }
    if (['failed', 'uncertain', 'interrupted', 'cancelled'].includes(last.state)) {
      output(`${stage}-failure.json`, last);
      throw new Error(`${stage} stopped with ${last.state}: ${last.safeReason || ''}`);
    }
    if (last.quality && ['failed', 'blocked', 'interrupted'].includes(last.quality.state)) {
      output(`${stage}-failure.json`, last);
      throw new Error(`${stage} quality ${last.quality.state}: ${last.quality.safeReason || ''}`);
    }
    if (last.publication && ['blocked', 'uncertain', 'cancelled'].includes(last.publication.state)) {
      output(`${stage}-failure.json`, last);
      throw new Error(`${stage} publication ${last.publication.state}: ${last.publication.safeReason || ''}`);
    }
    await delay(1000);
  }
  output(`${stage}-timeout.json`, last || {});
  throw new Error(`${stage} exceeded ${maxSeconds}s`);
}
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  await new Promise(resolve => server.close(resolve));
  return port;
}
await waitConsole();
const sourceCommit = command('git', ['rev-parse', 'HEAD']);
const checkout = path.join(evidence, 'trusted-source');
command('git', ['clone', '--local', '--no-hardlinks', '--no-checkout', root, checkout], 'source-clone.txt');
command('git', ['-C', checkout, 'remote', 'set-url', 'origin', fixtureRepository]);
command('git', ['-C', checkout, 'checkout', '-B', 'main', sourceCommit], 'source-checkout.txt');
const lockDigest = sha(fs.readFileSync(path.join(checkout, '.ayni.lock')));
const workspace = (await rpc('CreateWorkspace', { idempotencyKey: randomUUID(), displayName: 'Own-source managed self-development fixture' }, 'workspace.json')).workspace;
const project = (await rpc('CreateProject', { idempotencyKey: randomUUID(), workspaceId: workspace.id, displayName: 'Orchestrator source acceptance', goal: 'Prove own-source isolated execution, managed quality and local draft publication.' }, 'project.json')).project;
const conversation = (await rpc('CreateConversation', { idempotencyKey: randomUUID(), workspaceId: workspace.id, projectId: project.id, title: 'Reviewed README fixture change' }, 'conversation.json')).conversation;
const scope = { workspaceId: workspace.id, projectId: project.id, conversationId: conversation.id };
const plan = (await rpc('CreatePlan', { ...scope, idempotencyKey: randomUUID(), content: { title: 'Verify own-source managed task execution', objective: 'Exercise the deployed workflow against the exact Orchestrator source tree.', scope: 'One README documentation comment using an explicit deterministic coding fixture.', outOfScope: 'Live provider calls, live GitHub publication and merge.', constraints: ['Only README.md may be read by coding or changed.', 'Managed quality validates the full original repository snapshot.'], unresolvedQuestions: [], tasks: [{ key: 'readme', objective: 'Task self-development fixture', scope: 'Append one fixed documentation comment to README.md.', outOfScope: 'All other files and external services.', acceptanceCriteria: ['Only README.md changes.', 'Actual managed Ayni repository quality passes.', 'One local protocol-fixture draft is retained.'], repository: fixtureRepository, sourceRef: 'refs/heads/main', executionLimits: 'One explicitly scripted coding pass, 3600 seconds, 4096 tokens maximum.', unresolvedInputs: [], dependsOn: [], sourcePaths: ['README.md'] }] } }, 'plan.json')).version;
const approval = await rpc('ApprovePlan', { ...scope, idempotencyKey: randomUUID(), planId: plan.planId, expectedVersionId: plan.id, contentDigest: plan.contentDigest }, 'approval.json');
assert.equal(approval.tasks.length, 1);
const task = approval.tasks[0];
assert.deepEqual(task.specification.sourcePaths, ['README.md']);
const preparationSelection = { ...scope, taskId: task.id, idempotencyKey: randomUUID(), inputs: { repository: fixtureRepository, sourceRef: 'refs/heads/main', sourceCommit, runtimeImage: runtime, maxSeconds: 3600, maxTokens: 4096 } };
output('preparation-selection.json', preparationSelection);
const file = name => path.join(evidence, name);
const address = `127.0.0.1:${grpcPort}`;
command(binary('workflow-prepare-task'), ['verify', address, checkout, file('preparation-selection.json'), file('preparation-request.json')], 'preparation-verification.txt');
const preparation = JSON.parse(command(binary('workflow-prepare-task'), ['submit', address, file('preparation-request.json')], 'preparation-receipt.json')).preparation;
assert(preparation.attemptId && !preparation.blockers?.length);
assert.equal(preparation.inputs.sourceCommit, sourceCommit);
assert.equal(preparation.inputs.lockDigest, lockDigest);
command(binary('workflow-prepare-task'), ['start', address, checkout, file('preparation-request.json'), file('start-request.json')], 'start-verification.txt');
const start = JSON.parse(fs.readFileSync(file('start-request.json'), 'utf8'));
const pack = Buffer.from(start.sourcePack, 'base64');
assert(pack.length > 0 && pack.length <= 8 * 1024 * 1024);
command(binary('workflow-prepare-task'), ['submit-start', address, file('start-request.json')], 'start-receipt.json');
const selection = { ...scope, taskId: task.id, attemptId: preparation.attemptId };
const quality = await waitAttempt(selection, 'managed-quality', a => a.state === 'completed' && a.executionOutcome === 'completed' && a.quality?.state === 'passed', 3600);
assert.equal(quality.sourceCommit, sourceCommit);
assert.equal(quality.runtimeImage, runtime);
assert.match(quality.patch, /Verified through isolated Orchestrator task execution/);
assert.deepEqual([...quality.patch.matchAll(/^diff --git a\/(.+) b\/(.+)$/gm)].map(m => [m[1], m[2]]), [['README.md', 'README.md']]);
assert.equal(quality.quality.completedTargets, quality.quality.expectedTargets);
assert(quality.quality.artifactDigest && quality.quality.runtimeMetadataDigest && quality.quality.sourceFingerprint);
const imageContext = file('publication-image');
fs.mkdirSync(imageContext);
fs.writeFileSync(path.join(imageContext, 'source.pack'), pack);
fs.writeFileSync(path.join(imageContext, 'source.json'), JSON.stringify({ source_commit: sourceCommit }));
fs.writeFileSync(path.join(imageContext, 'Dockerfile'), `FROM ${fixtureBase}\nCOPY source.pack source.json /opt/fixture/\nENTRYPOINT ["python3","/opt/fixture/server.py","--directory","/var/lib/publication-fixture/self-development-${sourceCommit}-${preparation.attemptId}"]\n`);
const fixtureImage = `local/ayni-orchestrator-self-development-fixture:${sourceCommit.slice(0, 12)}-${randomUUID().slice(0, 8)}`;
command('docker', ['build', '--tag', fixtureImage, imageContext], 'publication-image-build.txt');
command('bash', ['-c', 'source "$1"; dev_load_image "$2"', 'self-development', path.join(root, 'dev/lib/images.sh'), fixtureImage], 'publication-image-load.txt');
command('helm', ['upgrade', '--install', 'ayni-orchestrator', path.join(root, 'dev/charts/ayni-orchestrator-local'), '--reset-then-reuse-values', '--kube-context', 'docker-desktop', '--namespace', namespace, '--set', 'ayni-orchestrator.taskExecution.fixtureEnabled=true', '--set', 'publicationFixture.enabled=true', '--set-string', `publicationFixture.image=${fixtureImage}`, '--set', 'ayni-orchestrator.publication.enabled=true', '--set-string', `ayni-orchestrator.publication.workspaceId=${workspace.id}`, '--set-string', `ayni-orchestrator.publication.repository=${fixtureRepository}`, '--set-string', 'ayni-orchestrator.publication.githubSecretName=publication-fixture-github', '--set', 'ayni-orchestrator.publication.fixtureEnabled=true', '--set-string', `ayni-orchestrator.publication.fixtureApiUrl=http://publication-fixture.${namespace}.svc:8080`, '--set-string', `ayni-orchestrator.publication.fixtureGitUrl=git://publication-fixture.${namespace}.svc:9418/fixture/repo.git`, '--wait', '--timeout=5m'], 'publication-binding.txt', { env: { ...process.env, HELM_NO_PLUGINS: '1' } });
output('publication-selection.json', selection);
command(binary('workflow-publish-task'), ['prepare', address, file('publication-selection.json'), file('publication-request.json')], 'publication-review.txt');
command(binary('workflow-publish-task'), ['submit', address, file('publication-request.json')], 'publication-receipt.json');
const published = await waitAttempt(selection, 'draft-publication', a => a.publication?.state === 'draft', 300);
assert.equal(published.publication.repository, fixtureRepository);
assert.equal(published.publication.patchDigest, quality.patchDigest);
assert.equal(published.publication.qualityDigest, quality.qualityDigest);
const fixturePort = await freePort();
const fixtureLog = fs.openSync(file('publication-forward.txt'), 'wx');
const fixtureForward = spawn('kubectl', ['--context', 'docker-desktop', '--namespace', namespace, 'port-forward', '--address', '127.0.0.1', 'service/publication-fixture', `${fixturePort}:8080`], { stdio: ['ignore', fixtureLog, fixtureLog] });
try {
  let remote;
  for (let n = 0; n < 100; n++) {
    try { const response = await fetch(`http://127.0.0.1:${fixturePort}/fixture/state`, { signal: AbortSignal.timeout(1000) }); if (response.ok) { remote = await response.json(); break; } } catch {}
    await delay(100);
  }
  assert(remote, 'publication fixture observation unavailable');
  output('publication-remote.json', remote);
  assert.equal(remote.create_calls, 1);
  assert.equal(remote.pulls.length, 1);
  const pull = remote.pulls[0];
  assert.equal(pull.head.sha, published.publication.commitSha);
  assert.equal(pull.base.sha, sourceCommit);
  assert.equal(pull.draft, true);
  assert(remote.references.includes(`refs/heads/${published.publication.branch} ${published.publication.commitSha}`));
} finally { fixtureForward.kill(); fs.closeSync(fixtureLog); }
const image = JSON.parse(command('docker', ['image', 'inspect', fixtureImage]))[0];
output('self-development.json', { schema: 'build.ayni.orchestrator.self-development-acceptance.v1', result: 'passed', actual_source_repository: 'https://github.com/ayni-oss/orchestrator.git', source_commit: sourceCommit, source_pack_digest: sha(pack), source_pack_bytes: pack.length, lock_digest: lockDigest, runtime_image: runtime, selected_source_paths: ['README.md'], coding: 'explicit deterministic fixture using actual Orchestrator source', quality: quality.quality, quality_digest: quality.qualityDigest, patch_digest: quality.patchDigest, publication: { protocol_fixture: true, declared_repository: fixtureRepository, image_id: image.Id, commit_sha: published.publication.commitSha, branch: published.publication.branch, pr_number: published.publication.prNumber }, live_provider: false, live_github: false, scope, task_id: task.id, attempt_id: preparation.attemptId });
