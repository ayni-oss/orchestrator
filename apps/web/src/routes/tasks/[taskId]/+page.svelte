<script lang="ts">
  import { enhance } from "$app/forms";
  import { ArrowLeft, RotateCcw, X } from "@lucide/svelte";
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

  let { data, form }: PageProps = $props();
  let pending = $state<string | undefined>();
  const terminalStates = ["succeeded", "failed", "cancelled", "deadline_exceeded", "unsupported"];
</script>

<svelte:head><title>{data.task?.snapshot.request.slice(0, 60) ?? "Task unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <Button href="/tasks" variant="ghost"><ArrowLeft aria-hidden="true" /> Development tasks</Button>
  {#if data.error}<Feedback kind="error" title="Task unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Task action failed" message={form.error.message} requestId={form.error.requestId} />{/if}
  {#if data.task}
    {@const task = data.task}
    {@const terminal = terminalStates.includes(task.state)}
    <PageHeader eyebrow={`Task ${task.id}`} title={task.snapshot.request} description="Task state is independent from candidate publication and exact-head assessment.">
      {#snippet actions()}<PollStatus />{/snippet}
    </PageHeader>
    <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card p-4"><div class="flex items-center gap-3"><StatusBadge status={task.state} tone={statusTone(task.state)} /><span class="text-xs text-muted-foreground">Updated {formatDate(task.updatedAt)}</span></div><div class="flex gap-2">
      {#if ["failed", "cancelled", "deadline_exceeded"].includes(task.state)}<form method="POST" action="?/retry" use:enhance={() => { pending = "retry"; return async ({ update }) => { pending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.retry} /><SubmitButton variant="outline" pending={pending === "retry"}><RotateCcw aria-hidden="true" /> Retry frozen snapshot</SubmitButton></form>{/if}
      {#if !terminal}<form method="POST" action="?/cancel" use:enhance={() => { pending = "cancel"; return async ({ update }) => { pending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.cancel} /><SubmitButton variant="destructive" pending={pending === "cancel"}><X aria-hidden="true" /> Cancel</SubmitButton></form>{/if}
    </div></div>

    <div class="grid gap-6 lg:grid-cols-2">
      <Card.Root><Card.Header><Card.Title>Frozen repository and runtime</Card.Title></Card.Header><Card.Content><dl class="space-y-4 text-sm"><div><dt class="text-muted-foreground">Repository revision</dt><dd><CodeValue value={task.snapshot.repository.revision} /></dd></div><div><dt class="text-muted-foreground">Profile revision</dt><dd><CodeValue value={task.snapshot.runtime.runtimeProfileRevisionId} /></dd></div><div><dt class="text-muted-foreground">OCI image</dt><dd><CodeValue value={`${task.snapshot.runtime.ociRepository}@${task.snapshot.runtime.resolvedDigest}`} /></dd></div><div><dt class="text-muted-foreground">Platform</dt><dd>{task.snapshot.runtime.platform.operatingSystem}/{task.snapshot.runtime.platform.architecture}</dd></div></dl></Card.Content></Card.Root>
      <Card.Root><Card.Header><Card.Title>Frozen request</Card.Title></Card.Header><Card.Content class="space-y-4 text-sm"><p><strong>Agent:</strong> {task.snapshot.agent.agent} · {task.snapshot.agent.provider}/{task.snapshot.agent.model}</p><div><h3 class="font-medium">Acceptance criteria</h3><ol class="mt-2 list-decimal pl-5">{#each task.snapshot.acceptanceCriteria as criterion}<li>{criterion}</li>{/each}</ol></div><p><strong>Deadline:</strong> {task.snapshot.deadlineAt ? formatDate(task.snapshot.deadlineAt) : "none"}</p><p><strong>Test edits:</strong> {task.snapshot.changeConstraints.allowTestEdits ? "authorized" : "protected"}</p></Card.Content></Card.Root>
    </div>
    <Card.Root><Card.Header><Card.Title>Candidate and pull request</Card.Title><Card.Description>Coding-time checks are advisory. The authoritative assessment runs separately against the exact PR head.</Card.Description></Card.Header><Card.Content class="space-y-4">
      {#each data.progress?.candidates ?? [] as candidate (candidate.id)}<div class="space-y-2"><div class="flex flex-wrap gap-3"><StatusBadge status={candidate.state} tone={statusTone(candidate.state)} />{#if candidate.head}<CodeValue value={candidate.head} />{/if}</div>{#if candidate.advisory}<p class="text-sm">{candidate.advisory}</p>{/if}</div>{:else}<p class="text-sm text-muted-foreground">No candidate is available yet.</p>{/each}
      {#each data.progress?.reviews ?? [] as review (review.id)}<div class="space-y-2"><div class="flex flex-wrap gap-3"><StatusBadge status={review.state} tone={statusTone(review.state)} />{#if review.url}<a class="underline" href={review.url} target="_blank" rel="noreferrer">Open pull request</a>{/if}</div>{#if review.failure}<p class="text-sm">{review.failure.message} {review.failure.remediation}</p>{/if}<p><a class="text-sm underline" href={`/reviews?review=${encodeURIComponent(review.id)}`}>View exact-head assessment and evidence</a></p></div>{:else}<p class="text-sm text-muted-foreground">{task.snapshot.publishDraft ? "Draft publication is requested and resumes automatically after coding succeeds, including after a restart." : "Automatic publication was not requested."}</p>{/each}
    </Card.Content></Card.Root>
    <Card.Root><Card.Header><Card.Title>Attempts and model usage</Card.Title></Card.Header><Card.Content class="space-y-3">
    {#each data.progress?.attempts ?? [] as attempt (attempt.id)}<div class="text-sm"><p>Attempt {attempt.attemptNumber} · {words(attempt.state)}</p><p class="text-muted-foreground">Input tokens: {attempt.usage.input ?? "unavailable"} · Output tokens: {attempt.usage.output ?? "unavailable"} · Interactions: {attempt.usage.interactions}</p>{#if attempt.failure}<p>{attempt.failure.message}</p>{/if}</div>{:else}<p class="text-sm">Waiting for the first attempt.</p>{/each}
    </Card.Content></Card.Root>
    {#if task.failure}<Feedback kind="error" title={words(task.failure.reason)} message={task.failure.message} />{/if}
  {/if}
</div>
