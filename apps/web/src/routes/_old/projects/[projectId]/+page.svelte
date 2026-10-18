<script lang="ts">
  import {page} from "$app/state";
  import {untrack} from "svelte";
  import {goto} from "$app/navigation";
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import TaskIdentity from "$lib/components/task-identity.svelte";
  import { enhance } from "$lib/forms.js";
  import type { PageProps } from "./$types";
  import {projectHref,projectViews} from "$lib/project-navigation.js";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import ProjectUpdates from "$lib/components/project-updates.svelte";
  import ProjectSettings from "$lib/components/project-settings.svelte";
  import ProjectPlans from "$lib/components/project-plans.svelte";
  import WorkerUpdates from "$lib/components/worker-updates.svelte";
  import ProjectConversation from "$lib/components/project-conversation.svelte";
  import NewTaskForm from "$lib/components/new-task-form.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { EllipsisVertical } from "@lucide/svelte";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { formatDate, statusTone } from "$lib/presentation.js";
  let { data }: PageProps = $props();
  let finishClosing: (()=>void) | undefined;
  async function closeTask() {
    if (!newTask) return;
    await new Promise<void>(resolve=>{finishClosing=resolve;newTask=false;});
  }
  let finishArchiving: (() => void) | undefined;
  async function closeArchive() {
    await new Promise<void>(resolve => { finishArchiving=resolve; archiveProject=false; });
  }
  let archiveProject=$state(false);
  let newTask=$state(untrack(()=>page.url.searchParams.get("directTask")==="1"));
</script>
<svelte:head><title>{data.project?.displayName ?? "Project unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class={["flex min-h-0 flex-col gap-5",data.view==="overview"&&"h-[calc(100svh-8rem)] min-h-[32rem]"]}>

  {#if data.project}
    <PageHeader eyebrow={data.project.state === "archived" ? "Archived project" : "Project"} title={data.project.displayName} description={data.project.goal}>{#snippet actions()}{#if data.project?.state === "active" && data.workspace?.state === "active"}
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>{#snippet child({ props })}<Button {...props} variant="ghost" size="icon-sm" aria-label={`${data.project.displayName} project actions`}><EllipsisVertical aria-hidden="true" /></Button>{/snippet}</DropdownMenu.Trigger>
        <DropdownMenu.Content align="end" preventScroll={false}>
          <DropdownMenu.Item disabled={!data.agentConfiguration} onSelect={()=>newTask=true}>Submit task directly</DropdownMenu.Item>
          <DropdownMenu.Item onSelect={() => archiveProject=true}>Archive</DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Root>{/if}{/snippet}</PageHeader>
    {#if data.workspace}<p class="text-sm text-muted-foreground">Workspace: <a class="underline" href={`/_old/workspaces/${encodeURIComponent(data.workspace.id)}`}>{data.workspace.displayName}</a>{#if data.workspace.state === "archived"} · Archived: new work is disabled.{/if}</p>{/if}
    {#if data.project.state==="archived"}<p>New tasks and retries are disabled for this archived project.</p>{/if}
    <ProjectUpdates {data} />
    {#if data.configurationError}<Feedback kind="warning" title="Agent configuration unavailable" message={data.configurationError.message} />{/if}
    <Tabs.Root value={data.view} onValueChange={view=>{void goto(projectHref(data.project!.id,view as typeof data.view,data.conversation?.id),{keepFocus:true,noScroll:true});}} class="min-h-0 flex-1 gap-5">
      <Tabs.List aria-label="Project views">{#each projectViews as view (view)}<Tabs.Trigger value={view}>{view==="overview"?"Conversation":view.charAt(0).toUpperCase()+view.slice(1)}</Tabs.Trigger>{/each}</Tabs.List>
      {#each projectViews as view (view)}
        <Tabs.Content value={view} class="min-h-0 data-[state=active]:flex data-[state=active]:flex-col">
          {#if data.view===view}
          {#if view==="settings"}
            {#key data.project.id}<ProjectSettings {data} />{/key}
          {:else if view==="plans"}
            <ProjectPlans {data} />
          {:else if view==="overview"}
            {#key `${data.project.id}-${data.conversation?.id??page.url.searchParams.get("new")??""}`}<ProjectConversation {data} />{/key}
          {:else}
    <section class="space-y-4"><h2 class="text-lg font-semibold">Tasks</h2><p class="text-sm text-muted-foreground">Open a task to inspect its execution attempts, quality results, and pull request.</p><div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      {#each data.tasks as task (task.id)}<Card.Root><Card.Header><Card.Title class="text-base"><TaskIdentity {task} /></Card.Title><Card.Description>{data.repositories.find(repo=>repo.id===task.snapshot.repository.repositoryConnectionId)?.displayName ?? "Removed repository · history retained"}</Card.Description></Card.Header><Card.Content class="space-y-3 text-sm"><StatusBadge status={task.state} tone={statusTone(task.state)} /><p class="text-muted-foreground">Created {formatDate(task.createdAt)}</p>        <p class="text-muted-foreground">{task.attemptCount} execution {task.attemptCount===1?"attempt":"attempts"} · Usage and PR results in task details</p></Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No tasks yet.</p>{/each}
    </div>{#if data.nextPageToken}<Button variant="outline" href={projectHref(data.project.id,"tasks",data.conversation?.id,data.nextPageToken)}>More tasks</Button>{/if}</section>
    {#if data.conversation}<WorkerUpdates updates={data.conversation.workerUpdates} />{/if}
          {/if}
          {/if}
        </Tabs.Content>
      {/each}
    </Tabs.Root>
    {#if data.project.state === "active"}
      <Dialog.Root bind:open={archiveProject} onOpenChangeComplete={open => { if(!open){finishArchiving?.();finishArchiving=undefined;} }}>
        <Dialog.Content>
          <Dialog.Header><Dialog.Title>Archive project</Dialog.Title><Dialog.Description>Archiving {data.project.displayName} prevents new tasks and retries. Existing work and history are retained.</Dialog.Description></Dialog.Header>
          <form use:enhance={() => async ({result,update}) => { if(result.type === "success") await closeArchive(); await update(); }} method="POST" action="?/archive">
            <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} />
            <Dialog.Footer><Button type="button" variant="outline" onclick={() => archiveProject=false}>Cancel</Button><Button type="submit">Archive project</Button></Dialog.Footer>
          </form>
        </Dialog.Content>
      </Dialog.Root>
    {/if}
    <Dialog.Root bind:open={newTask} onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>Submit task directly</Dialog.Title><Dialog.Description>Choose a repository and describe the work for {data.project.displayName}.</Dialog.Description></Dialog.Header><NewTaskForm project={data.project} repositories={data.repositories.filter(repository=>data.workspace?.repositoryConnectionIds.includes(repository.id))} profiles={data.profiles} models={data.models} connections={data.llmConnections} catalogues={data.connectionModels} provider={data.provider} configured={!!data.agentConfiguration} effectiveDefaults={data.agentConfiguration?.effective} idempotencyKey={data.taskKey} onSubmitted={closeTask} /></Dialog.Content></Dialog.Root>
  {/if}
</div>
