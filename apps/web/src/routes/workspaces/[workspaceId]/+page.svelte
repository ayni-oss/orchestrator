<script lang="ts">
  import {enhance} from "$lib/forms.js";
  import type {PageProps} from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import ProjectAgentSettings from "$lib/components/project-agent-settings.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Input} from "$lib/components/ui/input/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import {Label} from "$lib/components/ui/label/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  let {data}:PageProps=$props();
  let settingsError=$state(""),settingsSuccess=$state("");
  let saving=$state(false),archiving=$state(false),members=$state<string[]|undefined>();
  let finishArchiving:(()=>void)|undefined;
  async function closeArchive(){await new Promise<void>(resolve=>{finishArchiving=resolve;archiving=false;});}
</script>
<svelte:head><title>{data.workspace?.displayName??"Workspace unavailable"} · Ayni Orchestrator</title></svelte:head>
{#if data.workspace}
  <PageHeader eyebrow={data.workspace.state==="archived"?"Archived workspace":"Workspace"} title={data.workspace.displayName} description="Shared repositories and agent defaults for goal-oriented projects.">
    {#snippet actions()}{#if data.workspace?.state==="active"}<Button href={`/projects?workspace=${encodeURIComponent(data.workspace.id)}`}>Create project</Button><Button variant="outline" onclick={()=>archiving=true}>Archive workspace</Button>{/if}{/snippet}
  </PageHeader>
  {#if data.workspace.state==="archived"}<p>New work is disabled. Existing tasks and project history are retained.</p>{/if}
  <section class="space-y-4"><h2 class="text-lg font-semibold">Projects</h2><div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
    {#each data.projects as project (project.id)}<Card.Root><Card.Header><Card.Title><a class="hover:underline" href={`/projects/${encodeURIComponent(project.id)}`}>{project.displayName}</a></Card.Title><Card.Description>{project.goal}</Card.Description></Card.Header><Card.Content>{project.state}</Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No projects on this page.</p>{/each}
  </div>{#if data.nextPageToken}<Button variant="outline" href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`}>More projects</Button>{/if}</section>
  <section class="space-y-4" aria-label="Shared configuration"><h2 class="text-lg font-semibold">Shared configuration</h2><p class="text-sm text-muted-foreground">Workspace repositories, agent defaults, and planner idle defaults are shared across projects. Repository pages own runtimes and Ayni policy; provider connections are configured centrally.</p>
    <div class="flex flex-wrap gap-3 text-sm"><a class="underline" href="/source-control">Source control</a><a class="underline" href="/container-registries">Container registries</a><a class="underline" href="/llm-providers">LLM providers</a><a class="underline" href="/ayni">Ayni configuration</a></div>
    <div class="flex flex-wrap gap-3 text-sm" aria-label="Workspace repositories">{#each data.workspace.repositoryConnectionIds as id (id)}<a class="underline" href={`/repositories/${encodeURIComponent(id)}`}>{data.repositories.find(repository=>repository.id===id)?.displayName??id} · runtimes</a>{/each}</div>
  {#if data.workspace.state==="active"}
    <Card.Root><Card.Header><Card.Title>Workspace settings</Card.Title><Card.Description>Removing a repository prevents new work that uses it. Existing task snapshots and history are retained.</Card.Description></Card.Header><Card.Content>
      <form method="POST" action="?/update" use:enhance={()=>{saving=true;settingsError="";settingsSuccess="";return async({result,update})=>{try{if(result.type==="failure")settingsError=String(result.data?.error?.message??"Workspace settings could not be saved. Try again.");if(result.type==="success")settingsSuccess=String(result.data?.success??"Workspace settings saved.");await update({reset:false});}finally{saving=false;}};}} class="space-y-4">
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} />
        <Label for="workspace-name">Workspace name</Label><Input id="workspace-name" name="displayName" value={data.workspace.displayName} required maxlength={256} />
        <Label for="workspace-repositories">Repositories</Label><RepositoryPicker id="workspace-repositories" repositories={data.repositories} value={members??data.workspace.repositoryConnectionIds} onchange={value=>members=value} />
        <Label for="workspace-planner-timeout">Planner idle timeout</Label><ThemedSelect id="workspace-planner-timeout" name="plannerIdleTimeoutSeconds" value={String(data.workspace.plannerIdleTimeoutSeconds)} options={[{value:"300",label:"5 minutes"},{value:"86400",label:"24 hours"}]} required />
        <p class="text-sm text-muted-foreground">Keep an idle planner session ready for this long. Conversations can override this default. Saved history survives session expiry.</p>
        <Button type="submit" disabled={saving}>{saving?"Saving…":"Save workspace settings"}</Button>
        {#if settingsError}<Feedback kind="error" title="Workspace settings not saved" message={settingsError} />{:else if settingsSuccess}<p role="status" class="text-sm">{settingsSuccess}</p>{/if}
      </form>
    </Card.Content></Card.Root>
    {#key data.workspace.id}<ProjectAgentSettings project={data.workspace} connections={data.connections} catalogues={data.catalogues} idempotencyKey={data.idempotencyKey} />{/key}
    <Dialog.Root bind:open={archiving} onOpenChangeComplete={open=>{if(!open){finishArchiving?.();finishArchiving=undefined;}}}><Dialog.Content><Dialog.Header><Dialog.Title>Archive workspace</Dialog.Title><Dialog.Description>Archiving {data.workspace.displayName} prevents new work across its projects. Existing work and history are retained.</Dialog.Description></Dialog.Header><form method="POST" action="?/archive" use:enhance={()=>async({result,update})=>{if(result.type==="success")await closeArchive();await update();}}><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} /><Dialog.Footer><Button type="button" variant="outline" onclick={()=>archiving=false}>Cancel</Button><Button type="submit">Archive workspace</Button></Dialog.Footer></form></Dialog.Content></Dialog.Root>
  {:else}
    <section class="space-y-3"><h2 class="text-lg font-semibold">Repositories</h2>{#each data.workspace.repositoryConnectionIds as id (id)}<p><a class="underline" href={`/repositories/${encodeURIComponent(id)}`}>{data.repositories.find(repository=>repository.id===id)?.displayName??id}</a></p>{/each}</section>
  {/if}
  </section>
{/if}
