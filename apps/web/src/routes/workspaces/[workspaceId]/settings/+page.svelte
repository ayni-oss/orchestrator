<script lang="ts">
  import WorkspaceShell from "$lib/components/workspace-shell.svelte";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import {enhance} from "$lib/forms.js";
  import type {PageProps} from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import ProjectAgentSettings from "$lib/components/project-agent-settings.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Input} from "$lib/components/ui/input/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import {Label} from "$lib/components/ui/label/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  let {data}:PageProps=$props();

  let settingsError=$state(""),settingsSuccess=$state("");
  let saving=$state(false),archiving=$state(false);
  let finishArchiving:(()=>void)|undefined;
  async function closeArchive(){await new Promise<void>(resolve=>{finishArchiving=resolve;archiving=false;});}
</script>
<svelte:head><title>Workspace settings · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<WorkspaceShell {data} title="Settings">
{#if data.workspace}
  <section class="space-y-4" aria-label="Workspace settings">
    <div class="flex flex-wrap items-center justify-between gap-4">
      <h1 class="text-3xl font-semibold tracking-tight">Workspace settings</h1>
    </div><p class="text-sm text-muted-foreground">Projects inherit agent settings and planner defaults from this workspace.</p>
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
    <section class="space-y-4" aria-labelledby="workspace-danger-zone-title">
      <h2 id="workspace-danger-zone-title" class="text-lg font-semibold">Danger zone</h2>
      <InformationCard title="Archive workspace" danger>
        <InformationTable rows={[
          {label:"Disables",value:"New work across this workspace and its projects."},
          {label:"Keeps",value:"Existing tasks, projects, and conversation history."},
        ]} />
        <div class="flex justify-end"><Button variant="destructive" onclick={()=>archiving=true}>Archive workspace</Button></div>
      </InformationCard>
    </section>
    <Dialog.Root bind:open={archiving} onOpenChangeComplete={open=>{if(!open){finishArchiving?.();finishArchiving=undefined;}}}><Dialog.Content><Dialog.Header><Dialog.Title>Archive workspace</Dialog.Title><Dialog.Description>Archiving {data.workspace.displayName} prevents new work across its projects. Existing work and history are retained.</Dialog.Description></Dialog.Header><form method="POST" action="?/archive" use:enhance={()=>async({result,update})=>{if(result.type==="success")await closeArchive();await update();}}><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} /><Dialog.Footer><Button type="button" variant="outline" onclick={()=>archiving=false}>Cancel</Button><Button type="submit" variant="destructive">Archive workspace</Button></Dialog.Footer></form></Dialog.Content></Dialog.Root>
  {/if}
  </section>

{:else}<p role="alert">{data.error?.message ?? "Workspace unavailable."}</p>{/if}
</WorkspaceShell>
