<script lang="ts">
  import { ArrowRight, CircleDollarSign, FolderGit2, ListTodo, Server } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import { formatCost, formatDate, phaseTone } from "$lib/presentation.js";

  let { data }: PageProps = $props();
  const activePhases = ["queued", "coding", "waiting_for_input", "validation_queued", "validating", "pr_queued", "preparing_pr"];
  const activeTasks = $derived(data.tasks.filter((task) => activePhases.includes(task.phase)));
  const readyRepositories = $derived(data.repositories.filter((repository) => repository.latestPreparation?.status === "ready"));
  const recentCost = $derived.by(() => {
    if (data.tasks.length === 0) return "No recent tasks";
    const currency = data.tasks[0]?.modelUsage.estimatedCostCurrency;
    if (currency === undefined || data.tasks.some((task) => task.modelUsage.estimatedCostMicros === undefined || task.modelUsage.estimatedCostCurrency !== currency)) return "Partial";
    return formatCost(data.tasks.reduce((sum, task) => sum + BigInt(task.modelUsage.estimatedCostMicros ?? "0"), 0n).toString(), currency);
  });
</script>

<svelte:head>
  <title>Overview · Ayni Orchestrator</title>
  <meta name="description" content="Repository readiness and development task operations" />
</svelte:head>

<div data-bff-status={data.backend} data-grpc-provider={data.provider ?? "unavailable"} class="space-y-8">
  <PageHeader eyebrow="Operator workspace" title="Development control plane" description="Prepare exact repository revisions, supervise bounded development tasks, and inspect retained evidence from one authoritative workflow.">
    {#snippet actions()}
      <Button href="/tasks#new">Submit task <ArrowRight aria-hidden="true" /></Button>
    {/snippet}
  </PageHeader>

  {#if data.error}
    <Feedback kind={data.backend === "disconnected" ? "error" : "warning"} title={data.backend === "disconnected" ? "Control plane disconnected" : "Some data is unavailable"} message={data.error.message} requestId={data.error.requestId} />
  {/if}

  <section aria-label="Workspace status" class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
    <Card.Root>
      <Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Control plane</Card.Description><Server class="size-4 text-muted-foreground" aria-hidden="true" /></Card.Header>
      <Card.Content><StatusBadge status={data.backend} tone={data.backend === "connected" ? "success" : data.backend === "degraded" ? "warning" : "danger"} pulse={data.backend === "connected"} /><p class="mt-2 text-xs text-muted-foreground">Request {data.requestId}</p></Card.Content>
    </Card.Root>
    <Card.Root>
      <Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Ready repositories</Card.Description><FolderGit2 class="size-4 text-muted-foreground" aria-hidden="true" /></Card.Header>
      <Card.Content><p class="text-2xl font-semibold">{readyRepositories.length}<span class="text-sm font-normal text-muted-foreground"> / {data.repositories.length}</span></p></Card.Content>
    </Card.Root>
    <Card.Root>
      <Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Recent active tasks</Card.Description><ListTodo class="size-4 text-muted-foreground" aria-hidden="true" /></Card.Header>
      <Card.Content><p class="text-2xl font-semibold">{activeTasks.length}</p></Card.Content>
    </Card.Root>
    <Card.Root>
      <Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Recent estimated cost</Card.Description><CircleDollarSign class="size-4 text-muted-foreground" aria-hidden="true" /></Card.Header>
      <Card.Content><p class="text-lg font-semibold">{recentCost}</p><p class="mt-1 text-xs text-muted-foreground">Latest {data.tasks.length} task snapshots</p></Card.Content>
    </Card.Root>
  </section>

  <section aria-labelledby="recent-tasks" class="space-y-4">
    <div class="flex items-center justify-between gap-4"><div><h2 id="recent-tasks" class="text-lg font-semibold">Recent tasks</h2><p class="text-sm text-muted-foreground">Latest authoritative lifecycle snapshots.</p></div><Button href="/tasks" variant="outline">View all</Button></div>
    {#if data.tasks.length === 0}
      <EmptyState title="No development tasks" description="Submit a task after an exact repository revision is ready." />
    {:else}
      <div class="overflow-hidden rounded-lg border bg-card">
        <Table.Root>
          <Table.Header><Table.Row><Table.Head>Request</Table.Head><Table.Head>Repository</Table.Head><Table.Head>Status</Table.Head><Table.Head>Updated</Table.Head><Table.Head class="text-right">Cost</Table.Head></Table.Row></Table.Header>
          <Table.Body>
            {#each data.tasks.slice(0, 8) as task}
              <Table.Row>
                <Table.Cell class="max-w-xs"><a class="block truncate font-medium hover:underline" href={`/tasks/${task.id}`}>{task.request}</a><span class="font-mono text-xs text-muted-foreground">{task.id.slice(0, 8)}</span></Table.Cell>
                <Table.Cell>{task.repositoryOwner}/{task.repositoryName}</Table.Cell>
                <Table.Cell><StatusBadge status={task.phase} tone={phaseTone(task.phase)} /></Table.Cell>
                <Table.Cell class="whitespace-nowrap text-muted-foreground">{formatDate(task.updatedAt)}</Table.Cell>
                <Table.Cell class="whitespace-nowrap text-right">{formatCost(task.modelUsage.estimatedCostMicros, task.modelUsage.estimatedCostCurrency)}</Table.Cell>
              </Table.Row>
            {/each}
          </Table.Body>
        </Table.Root>
      </div>
    {/if}
  </section>
</div>
