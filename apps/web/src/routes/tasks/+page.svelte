<script lang="ts">
import type {PageProps} from "./$types";
import {goto} from "$app/navigation";
import {onMount} from "svelte";
import {subscribeRefresh} from "$lib/refresh.js";
import PageHeader from "$lib/components/page-header.svelte";
import EmptyState from "$lib/components/empty-state.svelte";
import CodeValue from "$lib/components/code-value.svelte";
import StatusBadge from "$lib/components/status-badge.svelte";
import TaskIdentity from "$lib/components/task-identity.svelte";
import {Button} from "$lib/components/ui/button/index.js";
import * as Table from "$lib/components/ui/table/index.js";
import {formatDate,statusTone,words} from "$lib/presentation.js";
let {data}:PageProps=$props();
const nextPageHref=$derived(data.nextPageToken?`/tasks?pageToken=${encodeURIComponent(data.nextPageToken)}`:undefined);
onMount(()=>{if(location.hash==="#new"&&!data.legacy)void goto("/tasks?new=1");return subscribeRefresh("app:tasks",{intervalMs:120_000});});
</script>
<svelte:head><title>Task history · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
<PageHeader eyebrow="Execution" title="Task history" description="Follow execution across projects. Discuss and approve new work within a project." />
<p><a class="underline" href="/projects">Choose a project to start work</a></p>
{#if data.legacy}<section aria-label="Choose a project"><h2 class="font-semibold">Choose a project to submit directly</h2>{#each data.projects as project (project.id)}<p><a class="underline" href={`/projects/${encodeURIComponent(project.id)}?directTask=1`}>{project.displayName}</a></p>{:else}<p>No active projects available.</p>{/each}</section>{/if}
    <section class="space-y-4"><div><h2 class="text-lg font-semibold">Execution records</h2><p class="text-sm text-muted-foreground">Status and worker phase show delivery progress. Quality and publication have separate outcomes.</p></div>
      {#if data.tasks.length === 0}<EmptyState title="No tasks submitted" description="Start a conversation in a project to plan work." />{:else}
        <div class="overflow-x-auto rounded-lg border bg-card"><Table.Root><Table.Header><Table.Row><Table.Head>Request</Table.Head><Table.Head>Status</Table.Head><Table.Head>Phase</Table.Head><Table.Head>Repository revision</Table.Head><Table.Head>Runtime digest</Table.Head><Table.Head>Updated</Table.Head></Table.Row></Table.Header><Table.Body>{#each data.tasks as task (task.id)}<Table.Row><Table.Cell><TaskIdentity {task} /></Table.Cell><Table.Cell><StatusBadge status={task.state} tone={statusTone(task.state)} /></Table.Cell><Table.Cell>{words(data.phases[task.id] ?? "unavailable")}</Table.Cell><Table.Cell><CodeValue value={task.snapshot.repository.revision.slice(0, 12)} label={task.snapshot.repository.revision} /></Table.Cell><Table.Cell><CodeValue value={task.snapshot.runtime.resolvedDigest.slice(0, 19)} label={task.snapshot.runtime.resolvedDigest} /></Table.Cell><Table.Cell>{formatDate(task.updatedAt)}</Table.Cell></Table.Row>{/each}</Table.Body></Table.Root></div>
        {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
      {/if}
    </section>
</div>
