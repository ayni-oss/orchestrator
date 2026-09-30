<script lang="ts">
  import { ArrowRight, Boxes, FolderGit2, ListTodo, Server } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import { formatDate, statusTone } from "$lib/presentation.js";

  let { data }: PageProps = $props();
  const activeStates = ["queued", "waiting_for_runtime", "running"];
  const activeTasks = $derived(data.tasks.filter((task) => activeStates.includes(task.state)));
  const readyProfiles = $derived(data.runtimeProfiles.filter((profile) => profile.state === "ready"));
</script>

<svelte:head><title>Overview · Ayni Orchestrator</title></svelte:head>

<div data-bff-status={data.backend} data-grpc-provider={data.provider ?? "unavailable"} class="space-y-8">
  <PageHeader eyebrow="Operator workspace" title="Development control plane" description="Connect repositories, reconcile runtime profiles, and observe frozen development tasks as independent resources.">
    {#snippet actions()}<Button href="/tasks#new">Submit task <ArrowRight aria-hidden="true" /></Button>{/snippet}
  </PageHeader>
  {#if data.error}<Feedback kind={data.backend === "disconnected" ? "error" : "warning"} title="Some control-plane data is unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}

  <section aria-label="Workspace status" class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
    <Card.Root><Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Control plane</Card.Description><Server class="size-4" /></Card.Header><Card.Content><StatusBadge status={data.backend} tone={data.backend === "connected" ? "success" : "warning"} /><p class="mt-2 text-xs text-muted-foreground">Request {data.requestId}</p></Card.Content></Card.Root>
    <Card.Root><Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Connections</Card.Description><FolderGit2 class="size-4" /></Card.Header><Card.Content><p class="text-2xl font-semibold">{data.repositoryConnections.length}</p></Card.Content></Card.Root>
    <Card.Root><Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Ready profiles</Card.Description><Boxes class="size-4" /></Card.Header><Card.Content><p class="text-2xl font-semibold">{readyProfiles.length}<span class="text-sm font-normal text-muted-foreground"> / {data.runtimeProfiles.length}</span></p></Card.Content></Card.Root>
    <Card.Root><Card.Header class="flex-row items-center justify-between pb-2"><Card.Description>Active tasks</Card.Description><ListTodo class="size-4" /></Card.Header><Card.Content><p class="text-2xl font-semibold">{activeTasks.length}</p></Card.Content></Card.Root>
  </section>

  <section aria-labelledby="recent-tasks" class="space-y-4">
    <div class="flex items-center justify-between"><div><h2 id="recent-tasks" class="text-lg font-semibold">Recent tasks</h2><p class="text-sm text-muted-foreground">Task completion is independent from publication and assessment.</p></div><Button href="/tasks" variant="outline">View all</Button></div>
    {#if data.tasks.length === 0}
      <EmptyState title="No development tasks" description="Submit work after a runtime profile has a verified artifact." />
    {:else}
      <div class="overflow-hidden rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Request</Table.Head><Table.Head>Runtime profile</Table.Head><Table.Head>Status</Table.Head><Table.Head>Updated</Table.Head></Table.Row></Table.Header><Table.Body>
        {#each data.tasks.slice(0, 8) as task}<Table.Row><Table.Cell><a class="font-medium hover:underline" href={`/tasks/${task.id}`}>{task.snapshot.request}</a></Table.Cell><Table.Cell class="font-mono text-xs">{task.snapshot.runtime.runtimeProfileRevisionId}</Table.Cell><Table.Cell><StatusBadge status={task.state} tone={statusTone(task.state)} /></Table.Cell><Table.Cell>{formatDate(task.updatedAt)}</Table.Cell></Table.Row>{/each}
      </Table.Body></Table.Root></div>
    {/if}
  </section>
</div>
