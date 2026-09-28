<script lang="ts">
  import { enhance } from "$app/forms";
  import { page } from "$app/state";
  import { ArrowLeft, ExternalLink, RotateCcw, Send, ShieldAlert } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import CodeValue from "$lib/components/code-value.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import TaskStages from "$lib/components/task-stages.svelte";
  import TaskUpdateStatus from "$lib/components/task-update-status.svelte";
  import { Button, buttonVariants } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { formatCost, formatDate, formatDuration, formatTokens, phaseTone, stageLabel, words } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let actionPending = $state<"cancel" | "retry" | "input" | undefined>();
  const cancellablePhases = ["queued", "coding", "waiting_for_input", "validation_queued", "validating", "pr_queued", "preparing_pr"];
  const retryablePhases = ["failed", "cancelled", "unsupported"];
  const notice = $derived(page.url.searchParams.get("notice"));
</script>

<svelte:head><title>{data.task ? `${data.task.input.request.slice(0, 60)} · Task` : "Task unavailable"} · Ayni Orchestrator</title></svelte:head>

<div class="space-y-8">
  <Button href="/tasks" variant="ghost"><ArrowLeft aria-hidden="true" /> Tasks</Button>
  {#if data.error}<PageHeader eyebrow="Task operations" title="Task unavailable" description="The requested task could not be loaded from the control plane." /><Feedback kind="error" title="Task unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Action failed" message={form.error.message} requestId={form.error.requestId} />{/if}
  {#if notice === "cancelled"}<Feedback kind="success" title="Cancellation requested" message="The control plane recorded cancellation intent. The lifecycle below remains authoritative." />{:else if notice === "retried"}<Feedback kind="success" title="Retry created" message="This linked task preserves the previous task and frozen evidence." />{:else if notice === "input-provided"}<Feedback kind="success" title="Follow-up submitted" message="A linked task was created with the requested operator input." />{/if}

  {#if data.task}
    {@const task = data.task}
    {@const terminal = !cancellablePhases.includes(task.phase)}
    <PageHeader eyebrow={`${task.repository.owner}/${task.repository.repository}`} title={task.input.request} description={`Task ${task.id}`}>
      {#snippet actions()}
        <TaskUpdateStatus taskId={task.id} resourceVersion={task.resourceVersion} {terminal} />
        {#if task.draftPrUrl}<Button href={task.draftPrUrl} target="_blank" rel="noreferrer">Open draft PR <ExternalLink aria-hidden="true" /></Button>{/if}
      {/snippet}
    </PageHeader>

    <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card p-4">
      <div class="flex items-center gap-3"><StatusBadge status={task.phase} tone={phaseTone(task.phase)} pulse={!terminal && task.phase !== "waiting_for_input"} /><span class="text-xs text-muted-foreground">Updated {formatDate(task.updatedAt)}</span></div>
      <div class="flex flex-wrap gap-2">
        {#if retryablePhases.includes(task.phase)}
          <form method="POST" action="?/retry" use:enhance={() => { actionPending = "retry"; return async ({ update }) => { actionPending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.retry} /><SubmitButton variant="outline" pending={actionPending === "retry"} pendingLabel="Retrying…"><RotateCcw aria-hidden="true" /> Retry</SubmitButton></form>
        {/if}
        {#if cancellablePhases.includes(task.phase)}
          <Dialog.Root>
            <Dialog.Trigger class={buttonVariants({ variant: "destructive" })}><ShieldAlert aria-hidden="true" /> Cancel task</Dialog.Trigger>
            <Dialog.Content preventScroll={false}>
              <Dialog.Header><Dialog.Title>Cancel this task?</Dialog.Title><Dialog.Description>This records durable cancellation intent and may stop active work. Completed external actions are not undone.</Dialog.Description></Dialog.Header>
              <Dialog.Footer>
                <Dialog.Close class={buttonVariants({ variant: "outline" })}>Keep task</Dialog.Close>
                <form method="POST" action="?/cancel" use:enhance={() => { actionPending = "cancel"; return async ({ update }) => { actionPending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.cancel} /><SubmitButton variant="destructive" pending={actionPending === "cancel"} pendingLabel="Cancelling…">Confirm cancellation</SubmitButton></form>
              </Dialog.Footer>
            </Dialog.Content>
          </Dialog.Root>
        {/if}
      </div>
    </div>

    <section aria-labelledby="stage-heading" class="space-y-3"><h2 id="stage-heading" class="text-lg font-semibold">Lifecycle stages</h2><TaskStages {task} /></section>

    {#if task.phase === "waiting_for_input"}
      <Card.Root class="border-amber-500/40">
        <Card.Header><Card.Title>Operator input requested</Card.Title><Card.Description>{task.requestedInput ?? "The coding worker requested additional information."}</Card.Description></Card.Header>
        <Card.Content><form method="POST" action="?/provideInput" class="space-y-3" use:enhance={() => { actionPending = "input"; return async ({ update }) => { actionPending = undefined; await update(); }; }}><input type="hidden" name="idempotencyKey" value={data.actionKeys.provideInput} /><div class="space-y-2"><Label for="follow-up">Response</Label><Textarea id="follow-up" name="input" required maxlength={8192} rows={4} /></div><SubmitButton pending={actionPending === "input"} pendingLabel="Submitting…"><Send aria-hidden="true" /> Submit follow-up</SubmitButton></form></Card.Content>
      </Card.Root>
    {/if}

    {#if task.failure}
      <Feedback kind="error" title={words(task.failure.reason)} message={task.failure.message} />
    {/if}
    {#if task.scopeViolations.length > 0}
      <Card.Root class="border-red-500/40"><Card.Header><Card.Title>Scope diagnostics</Card.Title><Card.Description>The backend rejected candidate changes outside the frozen authorization.</Card.Description></Card.Header><Card.Content><ul class="space-y-2 text-sm">{#each task.scopeViolations as violation}<li class="rounded bg-red-500/5 p-2">{violation}</li>{/each}</ul></Card.Content></Card.Root>
    {/if}

    <div class="grid gap-6 lg:grid-cols-2">
      <Card.Root>
        <Card.Header><Card.Title>Frozen scope</Card.Title><Card.Description>Immutable inputs captured when this task was accepted.</Card.Description></Card.Header>
        <Card.Content class="space-y-5 text-sm">
          <dl class="grid gap-4 sm:grid-cols-2">
            <div><dt class="text-xs text-muted-foreground">Start revision</dt><dd><CodeValue value={task.input.startRevision} /></dd></div>
            <div><dt class="text-xs text-muted-foreground">Preparation</dt><dd>{#if task.preparationId}<CodeValue value={task.preparationId} />{:else}Unavailable{/if}</dd></div>
            <div><dt class="text-xs text-muted-foreground">Provider / model</dt><dd>{task.model.provider} / {task.model.model}</dd></div>
            <div><dt class="text-xs text-muted-foreground">Deadline</dt><dd>{task.deadlineAt ? formatDate(task.deadlineAt) : "No explicit deadline"}</dd></div>
            <div><dt class="text-xs text-muted-foreground">Test edits</dt><dd>{task.input.changeConstraints.allowTestEdits ? "Authorized" : "Protected"}</dd></div>
            <div><dt class="text-xs text-muted-foreground">Ayni lock</dt><dd><CodeValue value={task.repository.lockSha256.slice(0, 16)} label={task.repository.lockSha256} /></dd></div>
          </dl>
          <div><h3 class="text-xs font-medium text-muted-foreground">Acceptance criteria</h3><ol class="mt-2 list-decimal space-y-1 pl-5">{#each task.input.acceptanceCriteria as criterion}<li>{criterion}</li>{/each}</ol></div>
          {#if task.providedInput}<div><h3 class="text-xs font-medium text-muted-foreground">Operator follow-up</h3><p class="mt-2 whitespace-pre-wrap rounded border bg-muted/30 p-3">{task.providedInput}</p></div>{/if}
          <div><h3 class="text-xs font-medium text-muted-foreground">Protected paths</h3>{#if task.input.changeConstraints.protectedPaths.length}<ul class="mt-2 flex flex-wrap gap-2">{#each task.input.changeConstraints.protectedPaths as path}<li><CodeValue value={path} /></li>{/each}</ul>{:else}<p class="mt-1 text-muted-foreground">No additional protected paths.</p>{/if}</div>
          {#if task.retryOfTaskId}<p class="text-xs text-muted-foreground">Retry of <a class="font-mono hover:underline" href={`/tasks/${task.retryOfTaskId}`}>{task.retryOfTaskId}</a></p>{/if}
        </Card.Content>
      </Card.Root>

      <Card.Root>
        <Card.Header><div class="flex items-start justify-between gap-3"><div><Card.Title>Provider usage</Card.Title><Card.Description>Retained token accounting and versioned estimated cost.</Card.Description></div><StatusBadge status={task.modelUsage.collectionStatus} tone={task.modelUsage.collectionStatus === "complete" ? "success" : task.modelUsage.collectionStatus === "collecting" ? "active" : "warning"} /></div></Card.Header>
        <Card.Content class="space-y-5">
          <div class="grid grid-cols-2 gap-3 sm:grid-cols-4"><div><p class="text-xs text-muted-foreground">Interactions</p><p class="text-xl font-semibold">{task.modelUsage.total.interactionCount}</p></div><div><p class="text-xs text-muted-foreground">Input</p><p class="text-xl font-semibold">{formatTokens(task.modelUsage.total.inputTokens)}</p></div><div><p class="text-xs text-muted-foreground">Output</p><p class="text-xl font-semibold">{formatTokens(task.modelUsage.total.outputTokens)}</p></div><div><p class="text-xs text-muted-foreground">Estimated cost</p><p class="text-base font-semibold">{formatCost(task.modelUsage.total.estimatedCostMicros, task.modelUsage.total.estimatedCostCurrency)}</p></div></div>
          {#if task.modelUsage.byStage.length > 0}<div class="space-y-2">{#each task.modelUsage.byStage as usage}<div class="flex items-center justify-between rounded border p-2 text-sm"><span>{words(usage.stage)}</span><span>{formatTokens(usage.summary.totalTokens)} tokens · {formatCost(usage.summary.estimatedCostMicros, usage.summary.estimatedCostCurrency)}</span></div>{/each}</div>{/if}
          <p class="text-xs text-muted-foreground">Pricing {task.modelUsage.total.pricingVersion ?? "unavailable"}. Estimates are informational and remain labeled by backend pricing version.</p>
        </Card.Content>
      </Card.Root>
    </div>

    <section aria-labelledby="timeline-heading" class="space-y-3">
      <div><h2 id="timeline-heading" class="text-lg font-semibold">Lifecycle timeline</h2><p class="text-sm text-muted-foreground">Durable, ordered events from the control plane.</p></div>
      {#if task.lifecycle.length === 0}<p class="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">No lifecycle events are retained yet.</p>{:else}<div class="overflow-x-auto rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Sequence</Table.Head><Table.Head>Time</Table.Head><Table.Head>Stage</Table.Head><Table.Head>Outcome</Table.Head><Table.Head>Reason</Table.Head></Table.Row></Table.Header><Table.Body>{#each task.lifecycle as event}<Table.Row><Table.Cell class="font-mono text-xs">{event.sequence}</Table.Cell><Table.Cell class="whitespace-nowrap">{formatDate(event.eventTime)}</Table.Cell><Table.Cell>{stageLabel(event.stage)}</Table.Cell><Table.Cell><StatusBadge status={event.outcome} tone={event.outcome === "succeeded" ? "success" : ["failed", "blocked", "cancelled", "unsupported"].includes(event.outcome) ? "danger" : "active"} /></Table.Cell><Table.Cell>{event.reasonCode ?? "—"}</Table.Cell></Table.Row>{/each}</Table.Body></Table.Root></div>{/if}
    </section>

    {#if task.modelUsage.interactions.length > 0}
      <section aria-labelledby="interactions-heading" class="space-y-3"><h2 id="interactions-heading" class="text-lg font-semibold">Provider interactions</h2><div class="overflow-x-auto rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Stage / model</Table.Head><Table.Head>Outcome</Table.Head><Table.Head>Tokens</Table.Head><Table.Head>Latency</Table.Head><Table.Head class="text-right">Cost</Table.Head></Table.Row></Table.Header><Table.Body>{#each task.modelUsage.interactions as interaction}<Table.Row><Table.Cell><span class="block">{words(interaction.stage)}</span><span class="text-xs text-muted-foreground">{interaction.resolvedModel ?? interaction.model}</span></Table.Cell><Table.Cell><StatusBadge status={interaction.outcome} tone={interaction.outcome === "succeeded" ? "success" : "danger"} /></Table.Cell><Table.Cell>{formatTokens(interaction.tokens.totalTokens)}</Table.Cell><Table.Cell>{formatDuration(interaction.latencySeconds)}</Table.Cell><Table.Cell class="text-right">{interaction.estimatedCost ? formatCost(interaction.estimatedCost.amountMicros, interaction.estimatedCost.currency) : "Unavailable"}</Table.Cell></Table.Row>{/each}</Table.Body></Table.Root></div></section>
    {/if}

    <section aria-labelledby="evidence-heading" class="space-y-3">
      <div><h2 id="evidence-heading" class="text-lg font-semibold">Retained evidence</h2><p class="text-sm text-muted-foreground">Structured validation results bound to the exact candidate and environment.</p></div>
      {#if data.evidenceError}<Feedback kind="warning" title="Evidence unavailable" message={data.evidenceError.message} requestId={data.evidenceError.requestId} />{/if}
      {#if data.evidence}
        {@const evidence = data.evidence}
        <Card.Root><Card.Header><div class="flex items-start justify-between gap-3"><div><Card.Title>Validation manifest</Card.Title><Card.Description><CodeValue value={evidence.manifestDigest} /></Card.Description></div><StatusBadge status={evidence.outcome} tone={evidence.outcome === "success" ? "success" : "danger"} /></div></Card.Header><Card.Content class="space-y-5">
          <dl class="grid gap-4 text-sm sm:grid-cols-2 lg:grid-cols-4"><div><dt class="text-xs text-muted-foreground">Check exit code</dt><dd>{evidence.checkExitCode}</dd></div><div><dt class="text-xs text-muted-foreground">Preparation reused</dt><dd>{evidence.preparationReused ? "Yes" : "No"}</dd></div><div><dt class="text-xs text-muted-foreground">Policy edited</dt><dd>{evidence.policyEdited ? "Yes" : "No"}</dd></div><div><dt class="text-xs text-muted-foreground">Tests edited</dt><dd>{evidence.testsEdited ? "Yes" : "No"}</dd></div><div class="sm:col-span-2"><dt class="text-xs text-muted-foreground">Candidate commit</dt><dd><CodeValue value={evidence.candidateCommit} /></dd></div><div class="sm:col-span-2"><dt class="text-xs text-muted-foreground">Lock digest</dt><dd><CodeValue value={evidence.lockSha256} /></dd></div></dl>
          <div class="grid gap-5 lg:grid-cols-2"><div><h3 class="text-sm font-medium">Changed paths ({evidence.changedPaths.length})</h3>{#if evidence.changedPaths.length}<ul class="mt-2 max-h-64 space-y-1 overflow-auto rounded border p-3 text-xs">{#each evidence.changedPaths as path}<li><CodeValue value={path} /></li>{/each}</ul>{:else}<p class="mt-2 text-sm text-muted-foreground">No changed paths retained.</p>{/if}</div><div><h3 class="text-sm font-medium">Artifacts ({evidence.artifacts.length})</h3>{#if evidence.artifacts.length}<ul class="mt-2 space-y-2">{#each evidence.artifacts as artifact}<li class="rounded border p-2 text-sm"><span class="block font-medium">{artifact.name}</span><CodeValue value={artifact.sha256} /></li>{/each}</ul>{:else}<p class="mt-2 text-sm text-muted-foreground">No artifacts retained.</p>{/if}</div></div>
          {#if evidence.scopeViolations.length}<div><h3 class="text-sm font-medium text-red-700 dark:text-red-300">Evidence scope violations</h3><ul class="mt-2 space-y-1 text-sm">{#each evidence.scopeViolations as violation}<li class="rounded bg-red-500/5 p-2">{violation}</li>{/each}</ul></div>{/if}
        </Card.Content></Card.Root>
      {:else}
        <div class="rounded-lg border border-dashed p-6"><p class="font-medium">Evidence not yet available</p><p class="mt-1 text-sm text-muted-foreground">Evidence appears after independent validation produces a retained manifest. This view does not manufacture a result from task phase.</p></div>
      {/if}
    </section>
  {/if}
</div>
