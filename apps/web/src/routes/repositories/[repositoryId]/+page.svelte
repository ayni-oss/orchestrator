<script lang="ts">
  import { enhance } from "$app/forms";
  import { ArrowLeft, RotateCw } from "@lucide/svelte";
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
  import { formatDuration, preparationTone } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let preparing = $state(false);
  const latest = $derived(form?.preparation ?? data.repository?.latestPreparation);
  const nextPageHref = $derived(data.nextPageToken === undefined || data.repository === undefined ? undefined : `/repositories/${encodeURIComponent(data.repository.enrollment.id)}?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>{data.repository ? `${data.repository.enrollment.repository} readiness` : "Repository unavailable"} · Ayni Orchestrator</title></svelte:head>

<div class="space-y-8">
  <Button href="/repositories" variant="ghost"><ArrowLeft aria-hidden="true" /> Repositories</Button>
  <PageHeader eyebrow="Repository readiness" title={data.repository ? `${data.repository.enrollment.owner}/${data.repository.enrollment.repository}` : "Repository unavailable"} description="Preparation is bound to the exact revision and committed Ayni lock. A ready result can be reused only when the backend confirms compatibility." />
  {#if data.error}<Feedback kind="error" title="Repository data unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Preparation failed" message={form.error.message} requestId={form.error.requestId} />{:else if form?.success}<Feedback kind="success" title="Preparation accepted" message={form.success} />{/if}

  {#if data.repository}
    <div class="grid gap-4 lg:grid-cols-[1fr_1.2fr]">
      <Card.Root>
        <Card.Header><Card.Title>Current readiness</Card.Title><Card.Description>The newest preparation known for this enrollment.</Card.Description></Card.Header>
        <Card.Content class="space-y-4">
          {#if latest}
            <div class="flex items-center justify-between gap-3"><StatusBadge status={latest.status} tone={preparationTone(latest.status)} pulse={["queued", "preparing"].includes(latest.status)} /><span class="text-xs text-muted-foreground">Reused {latest.reuseCount} times</span></div>
            <dl class="grid gap-3 text-sm sm:grid-cols-2"><div><dt class="text-xs text-muted-foreground">Revision</dt><dd><CodeValue value={latest.revision} /></dd></div><div><dt class="text-xs text-muted-foreground">Duration</dt><dd>{formatDuration(latest.durationSeconds)}</dd></div><div class="sm:col-span-2"><dt class="text-xs text-muted-foreground">Preparation ID</dt><dd><CodeValue value={latest.id} /></dd></div></dl>
            {#if latest.diagnostic}<div class="rounded-md border border-amber-500/30 bg-amber-500/5 p-3"><h3 class="text-sm font-medium">Diagnostic</h3><p class="mt-1 whitespace-pre-wrap text-sm text-muted-foreground">{latest.diagnostic}</p></div>{/if}
          {:else}
            <p class="text-sm text-muted-foreground">No revision has been prepared.</p>
          {/if}
        </Card.Content>
      </Card.Root>
      <Card.Root>
        <Card.Header><Card.Title>Prepare exact revision</Card.Title><Card.Description>Use the complete immutable Git commit SHA, not a branch or tag.</Card.Description></Card.Header>
        <Card.Content>
          <form method="POST" action="?/prepare" class="space-y-4" use:enhance={() => { preparing = true; return async ({ update }) => { preparing = false; await update(); }; }}>
            <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
            <div class="space-y-2"><Label for="revision">Git commit revision</Label><Input id="revision" name="revision" required minlength={40} maxlength={40} pattern={"[0-9a-f]{40}"} spellcheck="false" autocomplete="off" class="font-mono" placeholder="40-character commit SHA" /></div>
            <SubmitButton pending={preparing} pendingLabel="Queuing…"><RotateCw aria-hidden="true" /> Prepare revision</SubmitButton>
          </form>
        </Card.Content>
      </Card.Root>
    </div>

    <section aria-labelledby="preparation-history" class="space-y-4">
      <div><h2 id="preparation-history" class="text-lg font-semibold">Preparation history</h2><p class="text-sm text-muted-foreground">Retained outcomes and compatibility reuse for this enrollment.</p></div>
      {#if data.preparations.length === 0}
        <EmptyState title="No preparation history" description="Queue an exact revision to establish repository readiness." />
      {:else}
        <div class="overflow-hidden rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Revision</Table.Head><Table.Head>Status</Table.Head><Table.Head>Duration</Table.Head><Table.Head>Reuse</Table.Head><Table.Head>Diagnostic</Table.Head></Table.Row></Table.Header><Table.Body>
          {#each data.preparations as preparation}
            <Table.Row><Table.Cell><CodeValue value={preparation.revision.slice(0, 12)} label={preparation.revision} /></Table.Cell><Table.Cell><StatusBadge status={preparation.status} tone={preparationTone(preparation.status)} /></Table.Cell><Table.Cell>{formatDuration(preparation.durationSeconds)}</Table.Cell><Table.Cell>{preparation.reuseCount}</Table.Cell><Table.Cell class="max-w-md whitespace-normal text-sm text-muted-foreground">{preparation.diagnostic ?? "—"}</Table.Cell></Table.Row>
          {/each}
        </Table.Body></Table.Root></div>
        {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
      {/if}
    </section>
  {/if}
</div>
