<script>
  import { count, readable, consoleLink } from '../api.mjs';
  import { taskGuidance, taskStage, publicationLink, phaseElapsed } from '../execution.mjs';
  let { view, client, now } = $props();
  const selected = $derived(view.tasks.detail);
  const run = $derived(selected?.attempt);
  const timing = $derived(run?.timing);
  const publication = $derived(run?.publication);
  const link = $derived(publicationLink(publication));
  const traceId = $derived(/^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(run?.attemptId || '') ? run.attemptId.replaceAll('-', '') : '');
  const recorded = value => value ? new Date(value).toLocaleString() : 'Not recorded';
</script>
<details class="task-inspector" data-list-busy={view.tasks.busy}>
  <summary>Task execution & delivery</summary>
  <p class="muted">Coding, quality and publication have separate outcomes.</p>
  <button class="text-button" disabled={view.tasks.busy} onclick={() => client.tasks()}>Refresh tasks</button>
  {#if view.tasks.error}<p class="error" role="alert">Tasks unavailable: {view.tasks.error}</p>{/if}
  <div class="task-picker" aria-label="Admitted tasks">
    {#each view.tasks.items as task (task.id)}
      <button data-task-id={task.id} class:selected={view.tasks.selectedId === task.id} aria-pressed={view.tasks.selectedId === task.id} onclick={() => client.inspectTask(task.id)}>{task.proposedTaskKey}: {task.specification?.objective || task.id}</button>
    {:else}<p class="muted">{view.tasks.busy ? 'Loading admitted tasks…' : 'No admitted tasks in this conversation.'}</p>{/each}
  </div>
  {#if view.tasks.nextAfterId}<button class="text-button" disabled={view.tasks.busy} onclick={() => client.tasks(true)}>Load more tasks</button>{/if}
  {#if view.tasks.selectedId}
    <button class="text-button" disabled={view.tasks.detailBusy} onclick={() => client.inspectTask(view.tasks.selectedId, view.tasks.selectedAttemptId, true)}>{view.tasks.detailBusy ? 'Refreshing task…' : 'Refresh selected attempt'}</button>
  {/if}
  {#if view.tasks.detailError}<p class="error" role="alert">Task evidence unavailable: {view.tasks.detailError}</p>{/if}
  {#if selected}
    <section class="task-detail" aria-label="Selected task evidence" data-attempt-id={run?.attemptId || ''}>
      <h3>{selected.task?.specification?.objective || 'Task evidence'}</h3>
      <div class="current-stage"><span class="eyebrow">Current task phase</span><h3>{taskStage(run)}</h3></div>
      <p class="notice">{taskGuidance(selected.preparation, run)}</p>
      <a href={consoleLink(view.scope)}>Review source, recovery & publication ↗</a>
      <section aria-label="Preparation evidence"><h3>Preparation</h3>
        <dl><dt>Status</dt><dd>{!selected.preparation ? 'Awaiting preparation' : selected.preparation.attemptId ? 'Prepared' : 'Blocked'}</dd>
          <dt>Preparation recorded</dt><dd>{recorded(selected.preparation?.createdAt)}</dd>
          <dt>Blockers</dt><dd>{(selected.preparation ? (selected.preparation.blockers || []) : (selected.task?.executionBlockers || [])).join('\n') || 'None recorded'}</dd>
          <dt>Source commit</dt><dd>{run?.sourceCommit || selected.preparation?.inputs?.sourceCommit || 'Not recorded'}</dd>
          <dt>Runtime image</dt><dd>{run?.runtimeImage || selected.preparation?.inputs?.runtimeImage || 'Not recorded'}</dd></dl>
      </section>
      <section aria-label="Coding evidence"><h3>Coding</h3>
        <dl><dt>Status</dt><dd>{readable(run?.state || (selected.preparation?.attemptId ? 'not_started' : 'awaiting_preparation'))}</dd>
          <dt>Execution outcome</dt><dd>{readable(run?.executionOutcome || 'not_reported')}</dd><dt>Provider</dt><dd>{readable(run?.providerState || 'not_started')}</dd>
          <dt>Queue accepted</dt><dd>{recorded(timing?.queueAcceptedAt)}</dd>
          <dt>Claim accepted</dt><dd>{recorded(timing?.claimAcceptedAt)}</dd>
          <dt>Time awaiting claim</dt><dd>{phaseElapsed(timing?.queueAcceptedAt, timing?.claimAcceptedAt, run?.state === 'queued', now)}</dd>
          <dt>Coding result accepted</dt><dd>{recorded(timing?.codingResultAcceptedAt)}</dd>
          <dt>Since claim observation</dt><dd>{phaseElapsed(timing?.claimAcceptedAt, timing?.codingResultAcceptedAt || timing?.executionStoppedAt, run?.state === 'claimed', now)}</dd>
          <dt>Reason</dt><dd>{readable(run?.safeReason) || 'None recorded'}</dd></dl>
        {#if run?.parentAttemptId}<button class="text-button" onclick={() => client.inspectTask(view.tasks.selectedId, run.parentAttemptId)}>Inspect previous attempt</button>{/if}
        {#if run && ['queued', 'claimed', 'reporting'].includes(run.state)}<button disabled={!!view.pending || view.sending || view.busy} onclick={() => client.submit('CancelTaskExecution', { ...view.scope, taskId: view.tasks.selectedId, attemptId: run.attemptId })}>Cancel task execution</button><p class="muted">A provider request already sent may have incurred a charge.</p>{/if}
        {#if run}<details><summary>Accepted patch</summary><pre>{run.patch || 'No accepted changes.'}</pre></details>{/if}
      </section>
      <section aria-label="Quality evidence"><h3>Managed quality</h3>
        <dl><dt>Status</dt><dd>{readable(run?.quality?.state || 'not_reported')}</dd><dt>Reason</dt><dd>{readable(run?.quality?.safeReason) || 'None recorded'}</dd>
          <dt>Quality result accepted</dt><dd>{recorded(timing?.qualityResultAcceptedAt)}</dd>
          <dt>Since coding result accepted</dt><dd>{phaseElapsed(timing?.codingResultAcceptedAt, timing?.qualityResultAcceptedAt || timing?.executionStoppedAt, run?.state === 'reporting' && !run?.quality, now)}</dd>
          <dt>Completed targets</dt><dd>{run?.quality ? `${count(run.quality.completedTargets || '0')} / ${count(run.quality.expectedTargets || '0')}` : 'Not recorded'}</dd>
          <dt>Exit code</dt><dd>{run?.quality?.exitCode ?? 'Not recorded'}</dd><dt>Quality digest</dt><dd>{run?.qualityDigest || 'Not recorded'}</dd></dl>
        {#if run?.quality}<details><summary>Retained quality artifacts</summary><pre>{run.quality.artifact || 'No retained artifact content.'}</pre></details><details><summary>Quality identities</summary><pre>{JSON.stringify({ ...run.quality, artifact: undefined }, null, 2)}</pre></details>{/if}
      </section>
      <section aria-label="Publication evidence"><h3>Draft publication</h3>
        <dl><dt>Status</dt><dd>{readable(publication?.state || 'not_authorized')}</dd><dt>Reason</dt><dd>{readable(publication?.safeReason) || 'None recorded'}</dd>
          <dt>Authorization recorded</dt><dd>{recorded(timing?.publicationAuthorizedAt)}</dd><dt>Claim accepted</dt><dd>{recorded(timing?.publicationClaimAcceptedAt)}</dd>
          <dt>Result accepted</dt><dd>{recorded(timing?.publicationResultAcceptedAt)}</dd><dt>Since publication authorization</dt><dd>{phaseElapsed(timing?.publicationAuthorizedAt, timing?.publicationResultAcceptedAt, ['queued', 'claimed', 'prepared', 'creating', 'uncertain'].includes(publication?.state), now)}</dd>
          <dt>Branch</dt><dd>{publication?.branch || 'Not recorded'}</dd><dt>Last recorded PR state</dt><dd>{readable(publication?.prState) || 'Not recorded'}</dd></dl>
        {#if link}<a href={link} target="_blank" rel="noopener noreferrer">Open retained pull request ↗</a>{/if}
        {#if run && ['queued', 'claimed', 'prepared'].includes(publication?.state)}<button disabled={!!view.pending || view.sending || view.busy} onclick={() => client.submit('CancelTaskPublication', { ...view.scope, taskId: view.tasks.selectedId, attemptId: run.attemptId })}>Cancel draft publication</button>{/if}
      </section>
      {#if run}<section aria-label="Recovery evidence"><h3>Questions & recovery</h3>
        {#if view.tasks.recoveryError}<p class="error">Recovery evidence unavailable: {view.tasks.recoveryError}</p><button class="text-button" onclick={() => client.taskRecovery()}>Retry recovery evidence</button>{/if}
        {#each view.tasks.recovery as record (record.id)}
          <article><strong>{readable(record.kind)}</strong><p>{record.question?.text || record.answer?.text || record.proposal?.explanation}</p>
            {#if record.question}{@const answer = view.tasks.questionAnswers.find(item => item.answer?.questionId === record.id)}<p>{answer ? `Answer: ${answer.answer.text}` : 'Awaiting an answer.'}</p>{/if}
            {#if record.proposal}<p>{readable(record.proposal.action)} · {record.proposal.additionalSeconds} seconds · {record.proposal.additionalTokens} tokens proposed</p>{/if}
            <small>{record.author} · {recorded(record.createdAt)}</small>
          </article>
        {:else}<p class="muted">{view.tasks.recoveryBusy ? 'Loading recovery evidence…' : 'No recorded questions or recovery proposals in this page.'}</p>{/each}
        {#each view.tasks.confirmations as confirmation (confirmation.commandId)}<p>Confirmed recovery: <button class="text-button" onclick={() => client.inspectTask(view.tasks.selectedId, confirmation.attemptId)}>{confirmation.attemptId}</button></p>{/each}
        {#if view.tasks.recoveryNextAfterId}<button class="text-button" disabled={view.tasks.recoveryBusy} onclick={() => client.taskRecovery(true)}>Load more recovery records</button>{/if}
      </section>{/if}
      <details><summary>Attempt identities & diagnostics</summary><dl class="identities"><dt>Task</dt><dd>{view.tasks.selectedId}</dd><dt>Plan</dt><dd>{selected.task?.planId}</dd><dt>Attempt</dt><dd>{run?.attemptId || selected.preparation?.attemptId || 'Not recorded'}</dd><dt>Generation</dt><dd>{run?.generation || 'Not recorded'}</dd><dt>Pod</dt><dd>{run?.podUid || 'Not recorded'}</dd><dt>Diagnostic trace ID</dt><dd>{traceId || 'No trace retained'}</dd><dt>Evidence read at</dt><dd>{recorded(timing?.observedAt)}</dd></dl><p class="muted">Trace diagnostics do not replace accepted execution evidence. Stage intervals include waiting and handoff time.</p></details>
    </section>
  {/if}
</details>
