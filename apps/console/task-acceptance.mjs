// Browser-boundary fixtures extend the deployed console suite. Canonical authority,
// managed execution and publication transport are checked by cumulative acceptance.
import assert from 'node:assert/strict';

export async function exerciseTasks({ page, gateway, scope }) {
  const tasks = (await gateway(page, 'ListTasks', { ...scope, limit: 2 })).tasks;
  assert.equal(tasks.length, 2);
  await page.route('**/api/ListTasks', route => { const request = route.request().postDataJSON(); assert.equal(request.limit, 20); return route.fulfill({ json: request.afterId ? { tasks: [tasks[1]] } : { tasks: [tasks[0]], nextAfterId: tasks[0].id } }); });
  await page.locator('#task-refresh').click();
  await page.locator('#task-more').waitFor({ state: 'visible' });
  await page.locator('#task-more').click();
  await page.waitForFunction(() => document.querySelector('#task-list').children.length === 2);
  await page.unroute('**/api/ListTasks');
  await page.locator(`[data-inspect-task="${tasks[0].id}"]`).click();
  await page.waitForFunction(() => document.querySelector('#task-status').textContent.includes('Verify the approved source'));
  assert.match(await page.locator('#task-detail').innerText(), /awaiting preparation/);
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Download task selection' }).click();
  assert.equal((await download).suggestedFilename(), `task-${tasks[0].id}.json`);
  await page.locator('#task-auto-refresh').uncheck();
  const attemptId = crypto.randomUUID();
  const preparation = { attemptId, blockers: [], inputs: { repository: 'https://github.com/fixture/repo.git', sourceRef: 'refs/heads/main', sourceCommit: 'a'.repeat(40), runtimeImage: 'fixture@sha256:identity', maxSeconds: 120, maxTokens: 1000 } };
  let attempt = { attemptId, state: 'completed', executionOutcome: 'completed', providerState: 'completed', patch: '<img src=x onerror="window.taskInjected=true">', patchDigest: 'sha256:patch', qualityDigest: 'sha256:quality', quality: { state: 'passed', expectedTargets: '2', completedTargets: '2', exitCode: 0, artifact: '{"checks":"passed"}' } };
  let publicationStarts = [];
  let publicationCancels = [];
  let cancellationReceipt;
  let startReceipt;
  await page.route('**/api/GetTaskExecution', route => route.fulfill({ json: { preparation } }));
  await page.route('**/api/GetTaskAttempt', route => route.fulfill({ json: { attempt } }));
  const refresh = async () => {
    const response = page.waitForResponse(r => r.url().endsWith('/api/GetTaskAttempt'));
    await page.locator('#task-refresh').click(); await response;
    await page.waitForFunction(expected => { const d = document.querySelector('#task-status').dataset; return d.codingState === expected.state && d.qualityState === expected.quality && d.publicationState === expected.publication; }, { state: attempt.state, quality: attempt.quality?.state || '', publication: attempt.publication?.state || '' });
  };
  await refresh();
  await page.getByRole('button', { name: 'Review draft publication', exact: true }).waitFor();
  assert.match(await page.locator('#task-detail').innerText(), /2 \/ 2/);
  assert.match(await page.locator('#task-detail').innerText(), /Exit code\s*0/);
  await page.locator('[data-task-evidence=patch] summary').click();
  assert.equal(await page.locator('#task-detail img').count(), 0);
  assert.equal(await page.evaluate(() => window.taskInjected), undefined);
  await page.route('**/api/ReviewTaskPublication', route => {
    const selected = route.request().postDataJSON();
    return route.fulfill({ json: { request: { ...selected, expectedPatchDigest: attempt.patchDigest, expectedQualityDigest: attempt.qualityDigest }, title: 'feat(tasks): reviewed task', body: '<script>text only</script>', repository: preparation.inputs.repository, source_ref: 'refs/heads/main', source_commit: 'a'.repeat(40), branch: `task/${attemptId}`, base_branch: 'main', author_name: 'Ayni Orchestrator', author_email: 'orchestrator@ayni.build', commit_message: 'Reviewed commit' } });
  });
  await page.route('**/api/StartTaskPublication', async route => {
    const body = route.request().postDataJSON(); publicationStarts.push(body);
    startReceipt ||= { receipt: { commandId: crypto.randomUUID(), attemptId, publicationId: attemptId } };
    attempt = { ...attempt, publication: { state: 'queued', attemptId } };
    if (publicationStarts.length === 1) await route.abort('failed'); else await route.fulfill({ json: startReceipt });
  });
  await page.getByRole('button', { name: 'Review draft publication', exact: true }).click();
  await page.locator('#task-publication-consent').waitFor();
  assert.equal(await page.getByRole('button', { name: 'Authorize draft publication', exact: true }).isDisabled(), true);
  assert.equal(await page.locator('#task-publication-review script').count(), 0);
  await page.locator('#task-publication-consent').check();
  await page.getByRole('button', { name: 'Authorize draft publication', exact: true }).click();
  await page.locator('#error').waitFor({ state: 'visible' });
  const pending = await page.evaluate(() => JSON.parse(sessionStorage.getItem('pending')));
  assert.equal(pending.body.expectedPatchDigest, 'sha256:patch');
  assert.equal(pending.body.expectedQualityDigest, 'sha256:quality');
  await page.reload();
  await page.locator('#pending').waitFor({ state: 'visible' });
  assert.deepEqual(await page.evaluate(() => JSON.parse(sessionStorage.getItem('pending'))), pending);
  await page.locator('#retry').click();
  await page.locator('#pending').waitFor({ state: 'hidden' });
  assert.deepEqual(publicationStarts[0], publicationStarts[1]);
  await page.getByRole('button', { name: 'Cancel draft publication', exact: true }).waitFor();
  await page.route('**/api/CancelTaskPublication', async route => {
    publicationCancels.push(route.request().postDataJSON());
    cancellationReceipt ||= { receipt: { commandId: crypto.randomUUID(), attemptId } };
    attempt = { ...attempt, publication: { state: 'cancelled', attemptId } };
    if (publicationCancels.length === 1) await route.abort('failed'); else await route.fulfill({ json: cancellationReceipt });
  });
  await page.getByRole('button', { name: 'Cancel draft publication', exact: true }).click();
  await page.locator('#pending').waitFor({ state: 'visible' });
  await page.waitForFunction(() => !document.querySelector('#retry').disabled);
  await page.locator('#retry').click(); await page.locator('#pending').waitFor({ state: 'hidden' });
  assert.deepEqual(publicationCancels[0], publicationCancels[1]);
  await page.waitForFunction(() => document.querySelector('#task-status').textContent.includes('Publication was cancelled'));
  for (const state of ['creating', 'uncertain', 'draft', 'blocked']) {
    attempt = { ...attempt, publication: { state, repository: 'https://github.com/fixture/repo.git', prNumber: '12', prUrl: 'https://github.com/fixture/repo/pull/12', prState: 'closed' } };
    await refresh();
    assert.equal(await page.getByRole('button', { name: 'Cancel draft publication', exact: true }).count(), 0);
    assert.equal(await page.getByRole('button', { name: 'Review draft publication', exact: true }).count(), 0);
  }
  attempt = { ...attempt, publication: undefined, quality: { state: 'failed', safeReason: 'quality_check_failed', exitCode: 1 } };
  await refresh();
  await page.waitForFunction(() => document.querySelector('#task-status').textContent.includes('Quality did not pass'));
  assert.equal(await page.getByRole('button', { name: 'Review draft publication', exact: true }).count(), 0);
  attempt = { ...attempt, state: 'claimed', executionOutcome: '', quality: undefined, providerState: 'uncertain' };
  await refresh();
  await page.getByRole('button', { name: 'Cancel task execution', exact: true }).waitFor();
  assert.match(await page.locator('#task-status').innerText(), /may have incurred a charge/);
  let cancelBody;
  await page.route('**/api/CancelTaskExecution', route => {
    cancelBody = route.request().postDataJSON(); attempt = { ...attempt, state: 'cancelling' };
    return route.fulfill({ json: { receipt: { commandId: 'fixture-cancel', attemptId } } });
  });
  await page.getByRole('button', { name: 'Cancel task execution', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('#task-detail').textContent.includes('cancelling'));
  assert.equal(cancelBody.taskId, tasks[0].id); assert.equal(cancelBody.attemptId, attemptId);
  assert.equal(await page.getByRole('button', { name: 'Cancel task execution', exact: true }).count(), 0);
  // A response for a previous task cannot replace the new selection, even with
  // automatic refresh disabled. The in-flight read queues one successor read.
  let release;
  await page.route('**/api/GetTaskAttempt', async route => {
    if (route.request().postDataJSON().taskId === tasks[0].id) await new Promise(resolve => { release = resolve; });
    await route.fulfill({ json: { attempt } });
  });
  const oldRequest = page.waitForRequest(r => r.url().endsWith('/api/GetTaskAttempt'));
  await page.locator('#task-auto-refresh').uncheck();
  await page.locator('#task-refresh').click(); await oldRequest;
  await page.locator(`[data-inspect-task="${tasks[1].id}"]`).click();
  await page.waitForTimeout(100); release();
  await page.waitForFunction(id => document.querySelector('#task-status').dataset.taskId === id, tasks[1].id);
  assert.match(await page.locator('#task-detail').innerText(), new RegExp(tasks[1].specification.objective));
  // Restore real reads so the cumulative suite finishes on canonical state.
  for (const method of ['GetTaskAttempt', 'GetTaskExecution', 'ReviewTaskPublication', 'StartTaskPublication', 'CancelTaskPublication', 'CancelTaskExecution']) await page.unroute(`**/api/${method}`);
  // Reopen the task without the fixture-only pinned attempt before canonical reads.
  await page.locator(`[data-inspect-task="${tasks[1].id}"]`).click();
  await page.waitForFunction(() => document.querySelector('#task-status').textContent.includes('Verify the approved source'));
  console.log(JSON.stringify({ taskConsole: 'pass', cases: ['real-admitted-task-read', 'source-helper-selection', 'separate-outcomes', 'text-only-patch-and-preview', 'exact-publication-consent', 'lost-start-response-reload-retry-fixture', 'lost-cancel-response-retry-fixture', 'uncertainty-does-not-create-again', 'failed-quality-blocks-publication', 'coding-cancellation', 'stale-selection-response-fencing'] }));
}
