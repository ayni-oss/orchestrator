import assert from 'node:assert/strict';

// Extends the cumulative browser scenario against real PostgreSQL-backed APIs.
export async function exercisePlans({ page, context, endpoint, gateway, scope }) {
  const openEditor = tab => tab.locator('#plan-editor-details').evaluate(node => { node.open = true; });
  const field = (tab, name) => tab.locator(`#plan-editor > label [name="${name}"]`);
  const save = tab => tab.locator('#plan-editor > button[type="submit"]').click();
  await openEditor(page);
  const title = `Durable plan ${Date.now()} <script>window.planInjected=true</script>`;
  await field(page, 'title').fill(title);
  await field(page, 'objective').fill('Retain the project and originating conversation');
  await field(page, 'scope').fill('Durable plan review');
  await field(page, 'constraints').fill('No approval or execution');
  await field(page, 'unresolvedQuestions').fill('Which source commit should execution use?');
  await page.locator('#plan-add-task').click();
  await page.locator('#plan-tasks [name="key"]').fill('review');
  await page.locator('#plan-tasks [name="objective"]').fill('Keep task intent attached to the conversation');
  await page.locator('#plan-tasks [name="scope"]').fill('Only this plan');
  await page.locator('#plan-tasks [name="repository"]').fill('declared/example');
  await page.locator('#plan-tasks [name="unresolvedInputs"]').fill('Verify source and execution budget');
  // A lost response after commit must survive reload and replay the original command.
  let created;
  await page.route('**/api/CreatePlan', async route => {
    const accepted = await route.fetch(); assert.equal(accepted.status(), 200);
    created = await accepted.json(); await route.abort('failed');
  }, { times: 1 });
  await save(page);
  await page.locator('#pending').waitFor({ state: 'visible' });
  await page.locator('#error').waitFor({ state: 'visible' });
  await page.reload(); await page.locator('#pending').waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  assert.equal(await field(page, 'title').inputValue(), title);
  await page.locator('#retry').click(); await page.locator('#pending').waitFor({ state: 'hidden' });
  await page.locator('#plan-review h3').waitFor();
  assert.equal((await gateway(page, 'ListPlans', scope)).plans.length, 1);
  const planId = created.version.planId;
  assert.equal(created.version.projectId, scope.projectId);
  assert.equal(created.version.conversationId, scope.conversationId);
  assert.equal((await gateway(page, 'GetPlanVersion', { ...scope, planId })).version.id, created.version.id);
  assert.equal(await page.evaluate(() => window.planInjected), undefined);
  assert.match(await page.locator('#plan-review').innerText(), /unverified/);
  await page.locator('#plan-edit').click();
  await field(page, 'title').fill('First revision');
  const other = await context.newPage();
  await other.goto(endpoint);
  await other.evaluate(scope => sessionStorage.setItem('selection', JSON.stringify(scope)), scope);
  await other.reload();
  await other.locator(`[data-plan-id="${planId}"]`).click();
  await other.locator('#plan-edit').click();
  await field(other, 'title').fill('Unsaved competing revision');
  const acceptedRevision = page.waitForResponse(response => response.url().endsWith('/api/RevisePlan'));
  await save(page); assert.equal((await acceptedRevision).status(), 200);
  await other.waitForFunction(() => document.querySelector('#plan-list').textContent.includes('First revision'));
  assert.equal(await field(other, 'title').inputValue(), 'Unsaved competing revision');
  const conflict = other.waitForResponse(response => response.url().endsWith('/api/RevisePlan'));
  await save(other); assert.equal((await conflict).status(), 409);
  await other.locator('#pending').waitFor({ state: 'hidden' });
  assert.equal(await field(other, 'title').inputValue(), 'Unsaved competing revision');
  // Unsaved local edits persist across reconnect and reload, independently of reviewed versions.
  await other.locator('#toggle-watch').click(); await other.locator('#toggle-watch').click();
  await other.reload();
  await other.waitForFunction(() => document.querySelector('#connection').textContent === 'Connected');
  await openEditor(other);
  assert.equal(await field(other, 'title').inputValue(), 'Unsaved competing revision');
  const versions = await gateway(page, 'ListPlanVersions', { ...scope, planId, limit: 1 });
  assert.equal(versions.versions[0].ordinal, '2');
  const history = await gateway(page, 'ListPlanVersions', { ...scope, planId, beforeOrdinal: versions.nextBeforeOrdinal, limit: 1 });
  assert.equal(history.versions[0].id, created.version.id);
  await page.locator(`[data-plan-version="${created.version.id}"]`).click();
  await page.waitForFunction(title => document.querySelector('#plan-review h3').textContent === `${title} · version 1`, title);
  assert.equal((await gateway(page, 'GetPlanVersion', { ...scope, planId, versionId: created.version.id })).version.content.title, title);
  const foreign = (await gateway(page, 'CreateConversation', { ...scope, conversationId: undefined, title: 'Separate planning context', idempotencyKey: crypto.randomUUID() })).conversation;
  const denied = await page.evaluate(async body => {
    const result = await fetch('/api/GetPlanVersion', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' }, body: JSON.stringify(body) }); return result.status;
  }, { ...scope, conversationId: foreign.id, planId });
  assert.equal(denied, 404);
  const snapshot = (await gateway(page, 'GetConversationSnapshot', scope)).snapshot;
  assert.equal(snapshot.plans[0].currentOrdinal, '2');
  const retained = (await gateway(page, 'GetPlanVersion', { ...scope, planId })).version;
  await other.close();
  console.log(JSON.stringify({ planReview: 'pass', cases: ['lost-response-reload-retry', 'scope-origin', 'structured-task-inputs', 'escaping', 'concurrent-revision', 'stale-edit-conflict', 'unsaved-draft-reconnect-reload', 'immutable-history-pagination', 'snapshot-head'], retained: { scope, planId, version: retained } }));
  return { scope, planId, version: retained };
}
