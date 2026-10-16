<script lang="ts">
  import {page} from "$app/state";
  import CreateProject from "$lib/components/create-project.svelte";
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
  const view=$derived(["repositories","settings"].includes(page.url.searchParams.get("view")??"") ? page.url.searchParams.get("view") : "projects");
  let settingsError=$state(""),settingsSuccess=$state("");
  let saving=$state(false),archiving=$state(false),members=$state<{workspaceId:string;ids:string[]}|undefined>();
  let finishArchiving:(()=>void)|undefined;
  async function closeArchive(){await new Promise<void>(resolve=>{finishArchiving=resolve;archiving=false;});}
</script>
<svelte:head><title>{data.workspace?.displayName??"Workspace unavailable"} · Ayni Orchestrator</title></svelte:head>
{#if data.workspace}
  <PageHeader eyebrow={data.workspace.state==="archived"?"Archived workspace":"Workspace"} title={data.workspace.displayName} description="Shared repositories and agent defaults for goal-oriented projects.">
    {#snippet actions()}{#if data.workspace?.state==="active"}{#if view==="projects"}{#key data.workspace.id}<CreateProject {data} />{/key}{/if}{#if view==="settings"}<Button variant="outline" onclick={()=>archiving=true}>Archive workspace</Button>{/if}{/if}{/snippet}
  </PageHeader>
  {#if data.workspace.state==="archived"}<p>New work is disabled. Existing tasks and project history are retained.</p>{/if}
  <nav aria-label="Workspace views" class="flex gap-4 border-b pb-3 text-sm">
    {#each [{id:"projects",label:"Projects"},{id:"repositories",label:"Repositories"},{id:"settings",label:"Settings"}] as item (item.id)}
      <a href={`?view=${item.id}`} aria-current={view===item.id?"page":undefined} class={view===item.id?"font-semibold text-foreground":"text-muted-foreground hover:text-foreground"}>{item.label}</a>
    {/each}
  </nav>
  {#if view==="projects"}
  <section class="space-y-4"><h2 class="text-lg font-semibold">Projects</h2><div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
    {#each data.projects as project (project.id)}<Card.Root><Card.Header><Card.Title><a class="hover:underline" href={`/projects/${encodeURIComponent(project.id)}`}>{project.displayName}</a></Card.Title><Card.Description>{project.goal}</Card.Description></Card.Header><Card.Content>{project.state}</Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No projects yet. Create a project to define a goal and start work.</p>{/each}
  </div>{#if data.nextPageToken}<Button variant="outline" href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`}>More projects</Button>{/if}</section>
  {:else if view==="repositories"}
  <section class="space-y-4"><h2 class="text-lg font-semibold">Repositories</h2>
    <p class="text-sm text-muted-foreground">Projects can use these repositories. Runtime configuration is shared wherever a repository is used.</p>
    <Button variant="outline" href="/repositories">Open repository catalog</Button>
    {#if data.workspace.state==="active"}
      <form method="POST" action="?/update" use:enhance={()=>{saving=true;settingsError="";settingsSuccess="";return async({result,update})=>{try{if(result.type==="failure")settingsError=String(result.data?.error?.message??"Workspace settings could not be saved. Try again.");if(result.type==="success")settingsSuccess=String(result.data?.success??"Workspace settings saved.");await update({reset:false});}finally{saving=false;}};}} class="space-y-4 rounded-xl border p-5">
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
        <input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} />
        <input type="hidden" name="displayName" value={data.workspace.displayName} />
        <Label for="workspace-repositories">Add or remove existing repositories</Label>
        <RepositoryPicker id="workspace-repositories" repositories={data.repositories} value={members?.workspaceId===data.workspace.id?members.ids:data.workspace.repositoryConnectionIds} onchange={value=>members={workspaceId:data.workspace!.id,ids:value}} />
        <p class="text-sm text-muted-foreground">Connect new repositories in the global repository catalog. Removing a repository here prevents new work using it in this workspace and preserves existing history.</p>
        <Button type="submit" disabled={saving}>{saving?"Saving…":"Save repository membership"}</Button>
        {#if settingsError}<Feedback kind="error" title="Repository membership not saved" message={settingsError} />{:else if settingsSuccess}<p role="status">{settingsSuccess}</p>{/if}
      </form>
    {/if}
    <div class="grid gap-4 md:grid-cols-2">{#each data.workspace.repositoryConnectionIds as id (id)}
      <Card.Root><Card.Header><Card.Title><a class="hover:underline" href={`/repositories/${encodeURIComponent(id)}`}>{data.repositories.find(repository=>repository.id===id)?.displayName??id}</a></Card.Title><Card.Description>View shared runtimes and readiness</Card.Description></Card.Header></Card.Root>
    {/each}</div>
  </section>
  {:else}
  <section class="space-y-4" aria-label="Workspace settings"><h2 class="text-lg font-semibold">Settings</h2><p class="text-sm text-muted-foreground">Projects inherit agent settings and planner defaults from this workspace.</p>
  {#if data.workspace.state==="active"}
    <Card.Root><Card.Header><Card.Title>Workspace settings</Card.Title><Card.Description>Name this workspace and choose how long idle planner sessions remain ready.</Card.Description></Card.Header><Card.Content>
      <form method="POST" action="?/update" use:enhance={()=>{saving=true;settingsError="";settingsSuccess="";return async({result,update})=>{try{if(result.type==="failure")settingsError=String(result.data?.error?.message??"Workspace settings could not be saved. Try again.");if(result.type==="success")settingsSuccess=String(result.data?.success??"Workspace settings saved.");await update({reset:false});}finally{saving=false;}};}} class="space-y-4">
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} />
        <Label for="workspace-name">Workspace name</Label><Input id="workspace-name" name="displayName" value={data.workspace.displayName} required maxlength={256} />
        {#each data.workspace.repositoryConnectionIds as id (id)}<input type="hidden" name="repositoryConnectionIds" value={id} />{/each}
        <Label for="workspace-planner-timeout">Planner idle timeout</Label><ThemedSelect id="workspace-planner-timeout" name="plannerIdleTimeoutSeconds" value={String(data.workspace.plannerIdleTimeoutSeconds)} options={[{value:"300",label:"5 minutes"},{value:"86400",label:"24 hours"}]} required />
        <p class="text-sm text-muted-foreground">Keep an idle planner session ready for this long. Conversations can override this default. Saved history survives session expiry.</p>
        <Button type="submit" disabled={saving}>{saving?"Saving…":"Save workspace settings"}</Button>
        {#if settingsError}<Feedback kind="error" title="Workspace settings not saved" message={settingsError} />{:else if settingsSuccess}<p role="status" class="text-sm">{settingsSuccess}</p>{/if}
      </form>
    </Card.Content></Card.Root>
    {#key data.workspace.id}{#if data.configurationError}<p role="alert">{data.configurationError.message}</p>{/if}
    <ProjectAgentSettings capabilities={data.capabilities} configuration={data.agentConfiguration} project={data.workspace} connections={data.connections} catalogues={data.catalogues} idempotencyKey={data.idempotencyKey} />{/key}
    <Dialog.Root bind:open={archiving} onOpenChangeComplete={open=>{if(!open){finishArchiving?.();finishArchiving=undefined;}}}><Dialog.Content><Dialog.Header><Dialog.Title>Archive workspace</Dialog.Title><Dialog.Description>Archiving {data.workspace.displayName} prevents new work across its projects. Existing work and history are retained.</Dialog.Description></Dialog.Header><form method="POST" action="?/archive" use:enhance={()=>async({result,update})=>{if(result.type==="success")await closeArchive();await update();}}><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} /><Dialog.Footer><Button type="button" variant="outline" onclick={()=>archiving=false}>Cancel</Button><Button type="submit">Archive workspace</Button></Dialog.Footer></form></Dialog.Content></Dialog.Root>
  {/if}
  </section>
  {/if}
{/if}
