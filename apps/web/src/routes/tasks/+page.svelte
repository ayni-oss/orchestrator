<script lang="ts">
  import { enhance } from "$app/forms";
  import { ExternalLink, Filter, Play } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import CodeValue from "$lib/components/code-value.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { formatCost, formatDate, phaseTone, words } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let submitting = $state(false);
  const readyRepositories = $derived(data.repositories.filter((item) => item.latestPreparation?.status === "ready"));
  const nextPageHref = $derived.by(() => {
    if (data.nextPageToken === undefined) return undefined;
    const query = new URLSearchParams({ pageToken: data.nextPageToken });
    if (data.phase !== undefined) query.set("phase", data.phase);
    return `/tasks?${query.toString()}`;
  });
  const phases = ["queued", "coding", "waiting_for_input", "validation_queued", "validating", "pr_queued", "preparing_pr", "completed", "failed", "cancelled", "unsupported", "unknown"];
</script>

<svelte:head><title>Tasks · Ayni Orchestrator</title></svelte:head>

<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Task operations" title="Development tasks" description="Submit frozen work, monitor authoritative lifecycle state, and inspect usage and evidence without inferring backend transitions." />
  {#if data.error}<Feedback kind={data.backend === "disconnected" ? "error" : "warning"} title={data.backend === "disconnected" ? "Control plane disconnected" : "Some task data is unavailable"} message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Task submission failed" message={form.error.message} requestId={form.error.requestId} />{/if}

  <div class="grid items-start gap-6 xl:grid-cols-[minmax(0,1fr)_24rem]">
    <section aria-labelledby="task-list" class="min-w-0 space-y-4">
      <div class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div><h2 id="task-list" class="text-lg font-semibold">Task history</h2><p class="text-sm text-muted-foreground">{data.tasks.length} result{data.tasks.length === 1 ? "" : "s"}{data.phase ? ` in ${words(data.phase)}` : ""}</p></div>
        <form method="GET" class="flex items-end gap-2">
          <div class="space-y-1"><Label for="phase-filter" class="text-xs">Phase</Label><select id="phase-filter" name="phase" class="h-8 rounded-lg border bg-background px-2 text-sm" value={data.phase ?? ""}><option value="">All phases</option>{#each phases as phase}<option value={phase}>{words(phase)}</option>{/each}</select></div>
          <Button type="submit" variant="outline"><Filter aria-hidden="true" /> Apply</Button>
        </form>
      </div>
      {#if data.tasks.length === 0}
        <EmptyState title={data.phase ? "No tasks match this phase" : "No tasks submitted"} description={data.phase ? "Clear the filter or wait for a task to enter this phase." : "Use the submission form to start bounded development work."} />
      {:else}
        <div class="overflow-x-auto rounded-lg border bg-card">
          <Table.Root>
            <Table.Header><Table.Row><Table.Head>Task</Table.Head><Table.Head>Status</Table.Head><Table.Head>Revision</Table.Head><Table.Head>Updated</Table.Head><Table.Head class="text-right">Usage</Table.Head></Table.Row></Table.Header>
            <Table.Body>{#each data.tasks as task}<Table.Row>
              <Table.Cell class="max-w-sm"><a href={`/tasks/${task.id}`} class="block truncate font-medium hover:underline">{task.request}</a><span class="text-xs text-muted-foreground">{task.repositoryOwner}/{task.repositoryName} · {task.model.model}</span></Table.Cell>
              <Table.Cell><StatusBadge status={task.phase} tone={phaseTone(task.phase)} pulse={["coding", "validating", "preparing_pr"].includes(task.phase)} /></Table.Cell>
              <Table.Cell><CodeValue value={task.startRevision.slice(0, 10)} label={task.startRevision} /></Table.Cell>
              <Table.Cell class="whitespace-nowrap text-sm text-muted-foreground">{formatDate(task.updatedAt)}</Table.Cell>
              <Table.Cell class="whitespace-nowrap text-right text-sm">{formatCost(task.modelUsage.estimatedCostMicros, task.modelUsage.estimatedCostCurrency)}</Table.Cell>
            </Table.Row>{/each}</Table.Body>
          </Table.Root>
        </div>
        {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
      {/if}
    </section>

    <Card.Root id="new" class="xl:sticky xl:top-24">
      <Card.Header><Card.Title>Submit task</Card.Title><Card.Description>The selected repository, model, revision, criteria, and scope are frozen by the backend.</Card.Description></Card.Header>
      <Card.Content>
        {#if readyRepositories.length === 0 || data.models.length === 0}
          <Feedback kind="warning" title="Submission unavailable" message={readyRepositories.length === 0 ? "Enroll and prepare a repository first." : "No provider models are currently available."} />
        {/if}
        <form method="POST" action="?/create" class="mt-4 space-y-4" use:enhance={() => { submitting = true; return async ({ update }) => { submitting = false; await update(); }; }}>
          <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
          <div class="space-y-2"><Label for="task-repository">Repository</Label><select id="task-repository" name="repositoryId" required disabled={readyRepositories.length === 0} class="h-9 w-full rounded-lg border bg-background px-3 text-sm"><option value="" disabled selected>Select a repository</option>{#each readyRepositories as item}<option value={item.enrollment.id}>{item.enrollment.owner}/{item.enrollment.repository} · ready</option>{/each}</select></div>
          <div class="space-y-2"><Label for="task-model">Provider model</Label><select id="task-model" name="model" required disabled={data.models.length === 0} class="h-9 w-full rounded-lg border bg-background px-3 text-sm"><option value="" disabled selected>Select a model</option>{#each data.models as model}<option value={model.id}>{model.name ?? model.id}</option>{/each}</select><p class="text-xs text-muted-foreground">Provider: {data.provider ?? "unavailable"}</p></div>
          <div class="space-y-2"><Label for="task-revision">Exact start revision</Label><Input id="task-revision" name="startRevision" required minlength={40} maxlength={40} pattern={"[0-9a-f]{40}"} class="font-mono" autocomplete="off" spellcheck="false" /></div>
          <div class="space-y-2"><Label for="task-request">Request</Label><Textarea id="task-request" name="request" required maxlength={16384} rows={4} placeholder="Describe the bounded change and expected outcome." /></div>
          <div class="space-y-2"><Label for="task-criteria">Acceptance criteria</Label><Textarea id="task-criteria" name="acceptanceCriteria" required rows={4} placeholder="One criterion per line" aria-describedby="criteria-help" /><p id="criteria-help" class="text-xs text-muted-foreground">1–20 criteria, one per line.</p></div>
          <details class="rounded-lg border p-3"><summary class="cursor-pointer text-sm font-medium">Scope and deadline</summary><div class="mt-4 space-y-4"><div class="space-y-2"><Label for="protected-paths">Protected paths</Label><Textarea id="protected-paths" name="protectedPaths" rows={3} placeholder="One repository-relative path per line" /></div><label class="flex items-start gap-2 text-sm"><input type="checkbox" name="allowTestEdits" class="mt-0.5 size-4 rounded border-input" /><span><strong class="font-medium">Allow test edits</strong><span class="block text-xs text-muted-foreground">Authorization is frozen with the task scope.</span></span></label><div class="space-y-2"><Label for="deadline">Deadline seconds</Label><Input id="deadline" name="deadlineSeconds" type="number" min={1} max={86400} placeholder="Optional, up to 86400" /></div></div></details>
          <SubmitButton class="w-full" pending={submitting} pendingLabel="Submitting…" disabled={readyRepositories.length === 0 || data.models.length === 0}><Play aria-hidden="true" /> Submit development task</SubmitButton>
        </form>
      </Card.Content>
      <Card.Footer class="text-xs text-muted-foreground">Mutations use server-generated idempotency keys and native gRPC through the server BFF only. <ExternalLink class="ml-1 inline size-3" aria-hidden="true" /></Card.Footer>
    </Card.Root>
  </div>
</div>
