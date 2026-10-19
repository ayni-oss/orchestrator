<script lang="ts">
  import type {AgentCapabilitiesDto,AgentConfigurationDto} from "$lib/project-view.js";
  import AgentProfileForm from "$lib/components/agent-profile-form.svelte";
  import { tick, untrack } from "svelte";
  import type {ProjectViewData} from "$lib/project-view.js";
  import { enhance } from "$lib/forms.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import type {SubmitFunction} from "@sveltejs/kit";
  import Feedback from "$lib/components/feedback.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  let {project,connections,catalogues,idempotencyKey,inherited=false,workspaceId,capabilities,configuration}:{capabilities:AgentCapabilitiesDto|undefined;configuration:AgentConfigurationDto|undefined;project:{repositoryConnectionIds?:string[];resourceVersion:string;agentDefaults?:NonNullable<ProjectViewData["project"]>["agentDefaults"]};connections:ProjectViewData["llmConnections"];catalogues:ProjectViewData["connectionModels"];idempotencyKey:string;inherited?:boolean;workspaceId?:string}=$props();
  let projectConnection=$state(untrack(()=>configuration?.effective.projectConnectionId??"")),codingConnection=$state(untrack(()=>configuration?.effective.codingConnectionId??""));
  let projectModel=$state(untrack(()=>configuration?.effective.projectModel??"")),codingModel=$state(untrack(()=>configuration?.effective.codingModel??""));
  let pending=$state(false);
  let error=$state(""),success=$state("");
  const submit:SubmitFunction=({formData})=>{const inherit=formData.get("inherit")==="true";pending=true;error="";success="";return async({result,update})=>{try{if(result.type==="failure")error=String(result.data?.error?.message??"Settings could not be saved. Try again.");if(result.type==="success")success=String(result.data?.success??"Agent defaults saved.");await update({reset:false});if(inherit&&result.type==="success"){await tick();projectConnection=configuration?.effective.projectConnectionId??"";codingConnection=configuration?.effective.codingConnectionId??"";projectModel=configuration?.effective.projectModel??"";codingModel=configuration?.effective.codingModel??"";}}finally{pending=false;}};};
  const options=$derived(connections.map(c=>({value:c.id,label:c.displayName})));
</script>
<InformationCard title={workspaceId?"Project agent models":"Workspace agent defaults"}>
  <p class="text-sm text-muted-foreground">{workspaceId?"Model overrides affect this project. Use workspace defaults to share model choices.":"These research and coding models are inherited by projects unless overridden."}</p>
  {#if workspaceId}<p class="mb-4 text-sm text-muted-foreground">{inherited?"Inherited from":"Overrides for"} the <a class="underline" href={`/workspaces/${encodeURIComponent(workspaceId)}/settings`}>workspace defaults</a>.</p>{/if}
  <form method="POST" action="?/agentDefaults" use:enhance={submit} class="space-y-4">
    <input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="resourceVersion" value={project.resourceVersion} />
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="space-y-2"><Label for="project-agent-connection">Project agent connection</Label><ThemedSelect id="project-agent-connection" name="projectConnectionId" required value={projectConnection} onValueChange={value=>{projectConnection=value;projectModel="";}} options={options} placeholder="Choose connection" /><Label for="project-agent-model">Project agent model</Label><ThemedSelect id="project-agent-model" name="projectModel" required bind:value={projectModel} options={(catalogues[projectConnection]?.models??[]).map(m=>({value:m.id,label:m.id}))} placeholder="Choose available model" /></div>
      <div class="space-y-2"><Label for="coding-agent-connection">Coding agent connection</Label><ThemedSelect id="coding-agent-connection" name="codingConnectionId" required value={codingConnection} onValueChange={value=>{codingConnection=value;codingModel="";}} options={options} placeholder="Choose connection" /><Label for="coding-agent-model">Coding agent model</Label><ThemedSelect id="coding-agent-model" name="codingModel" required bind:value={codingModel} options={(catalogues[codingConnection]?.models??[]).map(m=>({value:m.id,label:m.id}))} placeholder="Choose available model" /></div>
    </div>
      {#if !connections.length}<p class="text-sm text-muted-foreground">Configure and test an <a href="/settings/llm-providers/openai" class="underline">OpenAI connection</a> before selecting models.</p>{:else if (projectConnection&&!catalogues[projectConnection])||(codingConnection&&!catalogues[codingConnection])}<p role="status" class="text-sm text-muted-foreground">Models could not be loaded for the selected connection. Check connection access and reload.</p>{/if}
    <Button type="submit" disabled={pending||!configuration||!projectConnection||!codingConnection||!projectModel||!codingModel}>{pending?"Saving…":"Save agent defaults"}</Button>
  </form>
  {#if workspaceId&&!inherited}<form method="POST" action="?/agentDefaults" use:enhance={submit} class="mt-3"><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}`} /><input type="hidden" name="resourceVersion" value={project.resourceVersion} /><input type="hidden" name="inherit" value="true" /><Button variant="outline" type="submit" disabled={pending||!configuration}>Use workspace defaults</Button></form>{/if}
{#if error}<Feedback kind="error" title="Agent defaults not saved" message={error} />{:else if success}<p role="status" class="text-sm">{success}</p>{/if}
</InformationCard>
<section class="space-y-4" aria-label="Agent capability profiles">
  <h3 class="text-lg font-semibold">Agent capability profiles</h3>
  <p class="text-sm text-muted-foreground">Choose the tools, limits, and instructions for each agent. Changes apply to new executions.</p>
  {#if capabilities&&configuration}
    <AgentProfileForm role="planner" {capabilities} configured={configuration.configured?.plannerProfile} inherited={configuration.inherited.plannerProfile} effective={configuration.effective.plannerProfile} repositories={project.repositoryConnectionIds??[]} project={!!workspaceId} {idempotencyKey} resourceVersion={project.resourceVersion} />
    <AgentProfileForm role="worker" {capabilities} configured={configuration.configured?.workerProfile} inherited={configuration.inherited.workerProfile} effective={configuration.effective.workerProfile} repositories={project.repositoryConnectionIds??[]} project={!!workspaceId} {idempotencyKey} resourceVersion={project.resourceVersion} />
  {:else}
    <InformationCard title="Agent profiles unavailable"><p role="status">Configuration unavailable. Reload before editing profiles.</p><Button disabled>Save planner profile</Button><Button disabled>Save worker profile</Button></InformationCard>
  {/if}
</section>
