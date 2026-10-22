// One explicitly enabled, bounded live call through an already configured session.
// Credentials remain in the session's existing scoped Kubernetes mount.
import assert from 'node:assert/strict';
import { chromium } from 'playwright';

assert.equal(process.env.AYNI_ALLOW_PAID_PROVIDER_SMOKE, '1', 'set AYNI_ALLOW_PAID_PROVIDER_SMOKE=1 to authorize one possibly paid turn');
const endpoint = new URL(process.env.CONSOLE_URL || 'http://127.0.0.1:3000');
assert.equal(endpoint.hostname, '127.0.0.1');
assert.equal(endpoint.protocol, 'http:');
const scope = {
  workspaceId: process.env.AYNI_SMOKE_WORKSPACE_ID,
  projectId: process.env.AYNI_SMOKE_PROJECT_ID,
  conversationId: process.env.AYNI_SMOKE_CONVERSATION_ID,
};
for (const value of Object.values(scope)) assert.match(value || '', /^[0-9a-f-]{36}$/i, 'supply an existing, isolated live-smoke scope');
const proposalSmoke = process.env.AYNI_SMOKE_PLAN_PROPOSAL === '1';
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.goto(endpoint.href);
  const api = async (method, body) => {
    const result = await page.evaluate(async ({ method, body }) => {
      const response = await fetch(`/api/${method}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body), signal: AbortSignal.timeout(15000) });
      return { status: response.status, body: await response.json() };
    }, { method, body });
    assert.equal(result.status, 200, `${method} failed; retain this turn's identity and inspect the console before taking another action`);
    return result.body;
  };
  const config = (await api('GetAgentConfiguration', { workspaceId: scope.workspaceId })).configuration;
  assert.equal(config.provider, 'openai', 'configure the workspace and scoped credential through the supported deployment first');
  assert.ok(config.model && config.model !== 'fixture-model', 'use an explicit live model');
  assert.ok(Number(config.maxTotalTokens) <= 32768 && Number(config.maxOutputTokens) <= 2048, 'bound the configured total allowance to 32768 tokens and output allowance to 2048 before this smoke');
  const snapshot = () => api('GetConversationSnapshot', scope).then(value => value.snapshot);
  const initial = await snapshot();
  assert.equal((initial.nonterminalTurns || []).length, 0, 'the smoke conversation must be idle');
  const calls = () => api('ListProviderCalls', scope).then(value => value.calls || []);
  const before = await calls();
  const idempotencyKey = crypto.randomUUID();
  console.log(JSON.stringify({ phase: 'admission', scope, idempotencyKey, totalTokenLimit: config.maxTotalTokens }));
  // Do not retry SendMessage, even after a lost response. Its key above permits
  // deliberate receipt recovery without introducing a fresh paid attempt.
  const admitted = (await api('SendMessage', { ...scope, idempotencyKey, content: proposalSmoke ? 'We have discussed the scope: a tiny documentation-only change to clarify a README. The constraint is no code or repository access. Call propose_plan exactly once as your sole function call and terminal result. Include one short task with key docs, objective clarify README, scope documentation, acceptance criterion clear wording, no repository or source claim, execution limit awaiting approval, no unresolved inputs. State the assumption that the operator will review the text and evidence that this conversation supplied the scope. No approval or execution.' : 'Use read_workspace_instructions, read_project_status and read_conversation_history once each, then reply with only: Smoke complete.' })).receipt;
  const until = Date.now() + 180000;
  let final;
  let accepted = false;
  while (Date.now() < until) {
    final = (await calls()).find(value => value.turnId === admitted.turnId);
    if (final?.state === 'uncertain') throw new Error(`Provider outcome uncertain for turn ${admitted.turnId}; preserve evidence, do not retry`);
    const state = await snapshot();
    const terminal = (state.recentTerminalTurns || []).find(turn => turn.id === admitted.turnId);
    if (terminal) {
      assert.equal(terminal.presentationState, 'PRESENTATION_STATE_COMPLETED', `turn ${admitted.turnId} failed: ${terminal.safeFailureReason || 'inspect retained evidence'}`);
      assert.equal(final?.state, 'completed');
      if (!proposalSmoke) assert.deepEqual([...new Set((final.toolInvocations || []).map(tool => tool.name))].sort(), ['read_workspace_instructions', 'read_project_status', 'read_conversation_history'].sort(), 'the live model must execute all three scoped reads for this smoke');
      let proposalVersion;
      if (proposalSmoke) {
        assert.equal((state.plans || []).length, (initial.plans || []).length + 1, 'one proposed plan is retained');
        const previous = new Set((initial.plans || []).map(plan => plan.id));
        const created = state.plans.find(plan => !previous.has(plan.id));
        proposalVersion = (await api('GetPlanVersion', { ...scope, planId: created.id })).version;
        assert.equal(proposalVersion.author, 'agent');
        assert.equal(proposalVersion.provenance.sourceTurnId, admitted.turnId);
        assert.equal(proposalVersion.provenance.providerResponseId, final.responseId);
        assert.ok(proposalVersion.content.tasks.length > 0);
      }
      const after = await calls();
      assert.equal(after.length, before.length + 1, 'one retained provider request lifecycle');
      await page.reload();
      assert.equal((await calls()).length, after.length, 'reconnect creates no new request lifecycle');
      if (proposalVersion) assert.deepEqual((await api('GetPlanVersion', { ...scope, planId: proposalVersion.planId })).version, proposalVersion, 'reconnect returns identical proposal and provenance');
      accepted = true;
      console.log(JSON.stringify({ outcome: 'passed', scope, turnId: admitted.turnId, callId: final.callId, state: final.state, reportedModel: final.reportedModel, inputTokens: final.inputTokens, outputTokens: final.outputTokens, totalTokens: final.totalTokens, proposalVersion }));
      break;
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  assert.equal(accepted, true, `turn ${admitted.turnId} did not finish within the smoke deadline; inspect it before another action`);
} finally { await browser.close(); }
