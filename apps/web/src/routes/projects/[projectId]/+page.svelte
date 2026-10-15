<script lang="ts">
  import { enhance } from "$lib/forms.js";
  import type { PageProps } from "./$types";
  import {projectHref,projectViews} from "$lib/project-navigation.js";
  import Feedback from "$lib/components/feedback.svelte";
  import {Textarea} from "$lib/components/ui/textarea/index.js";
  import AyniIcon from "$lib/components/ayni-icon.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import ProjectAgentSettings from "$lib/components/project-agent-settings.svelte";
  import ProjectUpdates from "$lib/components/project-updates.svelte";
  import ProjectConversation from "$lib/components/project-conversation.svelte";
  import NewTaskForm from "$lib/components/new-task-form.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
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
  let finishEditing: (() => void) | undefined;
  async function closeEditor() {
    await new Promise<void>(resolve => {finishEditing=resolve;editProject=false;});
  }
  let finishArchiving: (() => void) | undefined;
  async function closeArchive() {
    await new Promise<void>(resolve => { finishArchiving=resolve; archiveProject=false; });
  }
  let settingsError=$state("");
  let editProject=$state(false), archiveProject=$state(false), saving=$state(false);
  let newTask=$state(false), members=$state<string[]>([]), editing=$state(false);
  const selectedMembers=$derived(editing ? members : data.project?.repositoryConnectionIds ?? []);
</script>
<svelte:head><title>{data.project?.displayName ?? "Project unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">

  {#if data.project}
    <PageHeader eyebrow={data.project.state === "archived" ? "Archived project" : "Project"} title={data.project.displayName} description={data.project.goal}>{#snippet actions()}{#if data.project?.state === "active" && data.workspace?.state === "active"}<Button onclick={() => newTask=true}>New task</Button>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>{#snippet child({ props })}<Button {...props} variant="ghost" size="icon-sm" aria-label={`${data.project.displayName} project actions`}><EllipsisVertical aria-hidden="true" /></Button>{/snippet}</DropdownMenu.Trigger>
        <DropdownMenu.Content align="end" preventScroll={false}>
          <DropdownMenu.Item onSelect={() => { members=[...data.project!.repositoryConnectionIds]; editing=true; settingsError=""; editProject=true; }}>Project settings</DropdownMenu.Item>
          <DropdownMenu.Item onSelect={() => archiveProject=true}>Archive</DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Root>{/if}{/snippet}</PageHeader>
    {#if data.workspace}<p class="text-sm text-muted-foreground">Workspace: <a class="underline" href={`/workspaces/${encodeURIComponent(data.workspace.id)}`}>{data.workspace.displayName}</a>{#if data.workspace.state === "archived"} · Archived: new work is disabled.{/if}</p>{/if}
    {#if data.project.state==="archived"}<p>New tasks and retries are disabled for this archived project.</p>{/if}
    <nav class="flex gap-2 border-b pb-3" aria-label="Project views">
      {#each projectViews as view (view)}<Button variant={data.view===view?"secondary":"ghost"} href={projectHref(data.project.id,view,data.conversation?.id)} data-sveltekit-keepfocus aria-current={data.view===view?"page":undefined}>{view.charAt(0).toUpperCase()+view.slice(1)}</Button>{/each}
    </nav>
    <ProjectUpdates {data} />
    {#if data.view!=="tasks"}
      {#key `${data.project.id}-${data.conversation?.id??""}`}<ProjectConversation {data} />{/key}
    {:else}
    <section class="space-y-4"><h2 class="text-lg font-semibold">Tasks</h2><p class="text-sm text-muted-foreground">Open a task to inspect its execution attempts, quality results, and pull request.</p><div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      {#each data.tasks as task (task.id)}<Card.Root><Card.Header><Card.Title class="text-base"><a href={`/tasks/${encodeURIComponent(task.id)}`} class="hover:underline">{task.snapshot.request}</a></Card.Title><Card.Description>{data.repositories.find(repo=>repo.id===task.snapshot.repository.repositoryConnectionId)?.displayName ?? "Removed repository · history retained"}</Card.Description></Card.Header><Card.Content class="space-y-3 text-sm"><StatusBadge status={task.state} tone={statusTone(task.state)} /><p class="text-muted-foreground">Created {formatDate(task.createdAt)}</p>{@const proposal=data.conversation?.proposals.find(proposal=>proposal.taskId===task.id)}
        {@const version=data.conversation?.plan?.versions.find(version=>proposal&&version.proposalIds.includes(proposal.id))}
        {#if version}<a class="underline" href={projectHref(data.project.id,"plans",data.conversation?.id,undefined,version.version)}>Approved plan · version {version.version}</a>{/if}
        <p class="text-muted-foreground">{task.attemptCount} execution {task.attemptCount===1?"attempt":"attempts"} · Usage and PR results in task details</p></Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No tasks yet.</p>{/each}
    </div>{#if data.nextPageToken}<Button variant="outline" href={projectHref(data.project.id,"tasks",data.conversation?.id,data.nextPageToken)}>More tasks</Button>{/if}</section>
    {/if}
    {#if data.project.state === "active"}
      <Dialog.Root bind:open={editProject} onOpenChangeComplete={open=>{if(!open){finishEditing?.();finishEditing=undefined;}}}>
        <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
          <Dialog.Header><Dialog.Title>Project settings</Dialog.Title><Dialog.Description>The goal guides planning. Repositories and model overrides apply to this project. Shared runtime and connection settings belong to the workspace.</Dialog.Description></Dialog.Header>
          <form use:enhance={() => {saving=true;settingsError="";return async ({ result, update }) => {try{if(result.type === "failure")settingsError=String(result.data?.error?.message??"Settings could not be saved. Try again.");if(result.type === "success"){await closeEditor();editing=false;}await update({reset:false});}finally{saving=false;}};}} method="POST" action="?/update" class="space-y-3">
            <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} />
            <input type="hidden" name="workspaceId" value={data.project.workspaceId} />
            <Label for="goal">Goal and planning instructions</Label><Textarea id="goal" name="goal" value={data.project.goal} required maxlength={16384} rows={4} /><p class="text-xs text-muted-foreground">Describe the outcome and project constraints. Repository instructions such as AGENTS.md remain in each repository.</p>
            <Label for="name">Project name</Label><Input id="name" name="displayName" value={data.project.displayName} required maxlength={256} />
            <Label for="members">Repositories</Label><RepositoryPicker id="members" repositories={data.choices} value={selectedMembers} onchange={value => {members=value;editing=true;}} />
            <p class="text-xs text-muted-foreground">Removing a repository prevents new tasks and retries for it; task history is retained.</p>
            {#if settingsError}<Feedback kind="error" title="Settings not saved" message={settingsError} />{/if}
            <Dialog.Footer><Button type="button" variant="outline" onclick={() => editProject=false}>Close</Button><Button type="submit" disabled={saving}>{saving?"Saving…":"Save settings"}</Button></Dialog.Footer>
          </form>
          <section class="space-y-3" aria-label="Project repositories"><h2 class="font-semibold">Selected repositories</h2><p class="text-sm text-muted-foreground">Open a repository to inspect its shared runtimes.</p>
            {#each data.repositories as repository (repository.id)}<a class="flex items-center gap-2 break-all text-sm underline" href={`/repositories/${encodeURIComponent(repository.id)}`}>{repository.displayName}{#if repository.useAyni !== false}<AyniIcon class="size-4 shrink-0" role="img" aria-label="Ayni enabled" />{/if}</a>{/each}
          </section>
          {#if data.workspace?.state === "active"}{#key data.project.id}<ProjectAgentSettings project={data.project} inherited={!data.overrides} workspaceId={data.workspace.id} connections={data.llmConnections} catalogues={data.connectionModels} idempotencyKey={data.idempotencyKey} />{/key}{/if}
        </Dialog.Content>
      </Dialog.Root>
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
    <Dialog.Root bind:open={newTask} onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>New task</Dialog.Title><Dialog.Description>Choose a repository and describe the work for {data.project.displayName}.</Dialog.Description></Dialog.Header><NewTaskForm project={data.project} repositories={data.repositories.filter(repository=>data.workspace?.repositoryConnectionIds.includes(repository.id))} profiles={data.profiles} models={data.models} connections={data.llmConnections} catalogues={data.connectionModels} provider={data.provider} idempotencyKey={data.taskKey} onSubmitted={closeTask} /></Dialog.Content></Dialog.Root>
  {/if}
</div>
