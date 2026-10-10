<script lang="ts">
  import { enhance } from "$lib/forms.js";
  import { RotateCcw, X } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import CodeValue from "$lib/components/code-value.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import PollStatus from "$lib/components/poll-status.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { formatDate, statusTone, words } from "$lib/presentation.js";

  let { data }: PageProps = $props();
  const latestAttempt = $derived(data.progress?.attempts.at(-1));
  const canRetryPublication = $derived(Boolean(latestAttempt?.result?.commit && ["failed", "conflict"].includes(latestAttempt.result.publication)));
  let pending = $state<string | undefined>();
  const terminalStates = ["succeeded", "failed", "cancelled", "deadline_exceeded", "unsupported"];
</script>

<svelte:head><title>{data.task?.snapshot.request.slice(0, 60) ?? "Task unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <Button href="/tasks" variant="ghost">Development tasks</Button>

  {#if data.task}
    {@const task = data.task}
    {@const terminal = terminalStates.includes(task.state)}
    <PageHeader eyebrow={`Task ${task.id}`} title={task.snapshot.request} description="The worker codes, checks the full repository and publishes a draft with its actual quality result.">
      {#snippet actions()}<PollStatus scope="app:tasks" />{/snippet}
    </PageHeader>
    <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card p-4"><div class="flex items-center gap-3"><StatusBadge status={task.state} tone={statusTone(task.state)} /><span class="text-xs text-muted-foreground">Updated {formatDate(task.updatedAt)}</span></div><div class="flex gap-2">
      {#if ["failed", "cancelled", "deadline_exceeded"].includes(task.state)}<form method="POST" action="?/retry" use:enhance={() => { pending = "retry"; return async ({ update }) => { pending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.retry} /><SubmitButton variant="outline" pending={pending === "retry"}><RotateCcw aria-hidden="true" /> Retry task · new attempt</SubmitButton></form>{/if}
      {#if terminal && canRetryPublication}<form method="POST" action="?/retryPublication" use:enhance={() => { pending = "retryPublication"; return async ({ update }) => { pending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.retryPublication} /><SubmitButton variant="outline" pending={pending === "retryPublication"}><RotateCcw aria-hidden="true" /> Retry publication</SubmitButton></form>{/if}
      {#if !terminal}<form method="POST" action="?/cancel" use:enhance={() => { pending = "cancel"; return async ({ update }) => { pending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.cancel} /><SubmitButton variant="destructive" pending={pending === "cancel"}><X aria-hidden="true" /> Cancel</SubmitButton></form>{/if}
    </div></div>

    {#if task.failure}<Feedback kind="error" title="Task failed" message={task.failure.message} />{/if}
    <Card.Root><Card.Header><Card.Title>Progress</Card.Title><Card.Description>Worker stages and Rig model/tool activity. Logs are advisory and omit private reasoning, prompts, and command output.</Card.Description></Card.Header><Card.Content class="space-y-5">
      {#each data.progress?.attempts ?? [] as attempt (attempt.id)}
        <section class="space-y-2">
          <h3 class="text-sm font-medium">Attempt {attempt.attemptNumber}{attempt.publicationOnly ? " · publication retry" : ""} · {words(attempt.state)} · {words(attempt.progress?.phase ?? "unavailable")}</h3>
          <p class="text-sm">Turns consumed: {attempt.result?.codingTurnsConsumed ?? attempt.progress?.codingTurnsConsumed ?? "unavailable"} / {attempt.snapshot.limits.codingTurns} · Quality round: {attempt.result?.qualityRound ?? attempt.progress?.qualityRound ?? "unavailable"} · Repair limit: {attempt.snapshot.limits.qualityRepairLimit}</p>
          {#if attempt.result}<p class="text-sm">Execution: {words(attempt.result.execution)}</p>{/if}
          {#if attempt.failure}<p class="text-sm text-destructive">{attempt.failure.message}</p>{/if}
          <ol class="max-h-80 space-y-2 overflow-auto text-sm" aria-label={`Activity for attempt ${attempt.attemptNumber}`}>
            {#each attempt.agentActivity ?? [] as event (event.sequence)}
              <li class={event.failed ? "text-destructive" : "text-muted-foreground"}><time>{event.observedAt ? formatDate(event.observedAt) : "Time unavailable"}</time> · {event.message}{event.failed ? " — failed" : ""}</li>
            {:else}<li class="text-muted-foreground">No worker activity has been retained for this attempt yet.</li>{/each}
          </ol>
        </section>
      {:else}<p class="text-sm text-muted-foreground">Waiting for the first attempt.</p>{/each}
    </Card.Content></Card.Root>
    <div class="grid gap-6 lg:grid-cols-2">
      <Card.Root><Card.Header><Card.Title>Frozen repository and runtime</Card.Title></Card.Header><Card.Content><dl class="space-y-4 text-sm"><div><dt class="text-muted-foreground">Repository revision</dt><dd><CodeValue value={task.snapshot.repository.revision} /></dd></div><div><dt class="text-muted-foreground">Profile revision</dt><dd><CodeValue value={task.snapshot.runtime.runtimeProfileRevisionId} /></dd></div><div><dt class="text-muted-foreground">OCI image</dt><dd><CodeValue value={`${task.snapshot.runtime.ociRepository}@${task.snapshot.runtime.resolvedDigest}`} /></dd></div><div><dt class="text-muted-foreground">Platform</dt><dd>{task.snapshot.runtime.platform.operatingSystem}/{task.snapshot.runtime.platform.architecture}</dd></div></dl></Card.Content></Card.Root>
      <Card.Root><Card.Header><Card.Title>Frozen request</Card.Title></Card.Header><Card.Content class="space-y-4 text-sm"><p><strong>Agent:</strong> {task.snapshot.agent.agent} · {task.snapshot.agent.provider}/{task.snapshot.agent.model}</p><div><h3 class="font-medium">Acceptance criteria</h3><ol class="mt-2 list-decimal pl-5">{#each task.snapshot.acceptanceCriteria as criterion}<li>{criterion}</li>{/each}</ol></div><p><strong>Agent iterations:</strong> {task.snapshot.limits.codingTurns}</p><p><strong>Deadline:</strong> {task.snapshot.deadlineAt ? formatDate(task.snapshot.deadlineAt) : "none"}</p><p><strong>Test edits:</strong> {task.snapshot.changeConstraints.allowTestEdits ? "authorized" : "protected"}</p></Card.Content></Card.Root>
    </div>
    <Card.Root><Card.Header><Card.Title>Quality</Card.Title><Card.Description>Mandatory full-repository checks bind the final committed source.</Card.Description></Card.Header><Card.Content class="space-y-5">
      {#each data.progress?.attempts ?? [] as attempt (attempt.id)}
        {@const quality = attempt.result?.quality ?? attempt.progress?.latestFullCheck}
        <section class="space-y-2"><h3 class="font-medium">Attempt {attempt.attemptNumber} · {words(quality?.outcome ?? "unavailable")}</h3>
          <p class="text-sm">Completed targets: {quality?.completedTargets ?? "unavailable"} · Skipped targets: {quality?.skippedTargets ?? "unavailable"} · Warnings: {quality?.warningOffenders ?? "unavailable"} · Failures: {quality?.failingOffenders ?? "unavailable"}</p>
          {#if quality?.evidence}<p class="text-sm">Checked commit: <CodeValue value={quality.evidence.commit} /></p><p class="text-sm">Evidence digest: <CodeValue value={quality.evidence.signalsDigest} /></p>{/if}
          <ul class="space-y-1 text-sm" aria-label="Quality signals">{#each quality?.rows ?? [] as row, index (`${row.kind}:${index}`)}<li>{row.kind}: {row.passed ? "passed" : "failed"}</li>{/each}</ul>
          <ul class="space-y-1 text-sm" aria-label="Actionable findings">{#each quality?.findings ?? [] as finding, index (index)}<li>{finding}</li>{/each}</ul>
        </section>
      {:else}<p class="text-sm text-muted-foreground">Full quality result unavailable.</p>{/each}
    </Card.Content></Card.Root>
    <Card.Root><Card.Header><Card.Title>Pull request</Card.Title><Card.Description>The worker delivers retained final work as a draft and records the publication receipt.</Card.Description></Card.Header><Card.Content class="space-y-4">
      {#each data.progress?.attempts ?? [] as attempt (attempt.id)}
        {@const receipt = attempt.pullRequest ?? attempt.result?.pullRequest}
        {@const publication = attempt.result?.publication ?? attempt.progress?.publication ?? "unavailable"}
        <section class="space-y-2"><p>Attempt {attempt.attemptNumber} · {receipt ? attempt.result?.quality?.outcome === "failed" ? "Draft published · quality checks failed" : attempt.result?.quality?.outcome === "passed" ? "Draft published · quality checks passed" : "Draft published · quality checks unavailable" : `Publication ${words(publication)}`}</p>
          {#if receipt?.url}<a class="underline" href={receipt.url} target="_blank" rel="noreferrer">Open pull request #{receipt.number}</a>{/if}
          {#if receipt}<p class="text-sm">Head: <CodeValue value={receipt.headCommit} /></p>{/if}
        </section>
      {:else}<p class="text-sm text-muted-foreground">Waiting for worker delivery.</p>{/each}
    </Card.Content></Card.Root>
    <Card.Root><Card.Header><Card.Title>Attempts and model usage</Card.Title></Card.Header><Card.Content class="space-y-3">
    {#each data.progress?.attempts ?? [] as attempt (attempt.id)}<div class="text-sm"><p>Attempt {attempt.attemptNumber} · {words(attempt.state)}</p><p class="text-muted-foreground">Input tokens: {attempt.usage.input ?? "unavailable"} · Output tokens: {attempt.usage.output ?? "unavailable"} · Interactions: {attempt.usage.interactions ?? "unavailable"}</p>{#if attempt.failure}<p>{attempt.failure.message}</p>{/if}</div>{:else}<p class="text-sm">Waiting for the first attempt.</p>{/each}
    </Card.Content></Card.Root>
  {/if}
</div>
