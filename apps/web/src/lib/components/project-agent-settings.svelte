<script lang="ts">
  import AgentProfileFields from "$lib/components/agent-profile-fields.svelte";
  import { tick, untrack } from "svelte";
  import type { PageData } from "../../routes/projects/[projectId]/$types";
  import { enhance } from "$lib/forms.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import type {SubmitFunction} from "@sveltejs/kit";
  import Feedback from "$lib/components/feedback.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  let {project,connections,catalogues,idempotencyKey,inherited=false,workspaceId,configuredDefaults}:{project:{repositoryConnectionIds?:string[];resourceVersion:string;agentDefaults?:NonNullable<PageData["project"]>["agentDefaults"]};connections:PageData["llmConnections"];catalogues:PageData["connectionModels"];idempotencyKey:string;inherited?:boolean;workspaceId?:string;configuredDefaults?:NonNullable<PageData["project"]>["agentDefaults"]}=$props();
  let projectConnection=$state(untrack(()=>project.agentDefaults?.projectConnectionId??"")),codingConnection=$state(untrack(()=>project.agentDefaults?.codingConnectionId??""));
  let projectModel=$state(untrack(()=>project.agentDefaults?.projectModel??"")),codingModel=$state(untrack(()=>project.agentDefaults?.codingModel??""));
  let pending=$state(false);
  let error=$state(""),success=$state("");
  const submit:SubmitFunction=({formData})=>{const inherit=formData.get("inherit")==="true";pending=true;error="";success="";return async({result,update})=>{try{if(result.type==="failure")error=String(result.data?.error?.message??"Settings could not be saved. Try again.");if(result.type==="success")success=String(result.data?.success??"Agent defaults saved.");await update({reset:false});if(inherit&&result.type==="success"){await tick();projectConnection=project.agentDefaults?.projectConnectionId??"";codingConnection=project.agentDefaults?.codingConnectionId??"";projectModel=project.agentDefaults?.projectModel??"";codingModel=project.agentDefaults?.codingModel??"";}}finally{pending=false;}};};
  const options=$derived(connections.map(c=>({value:c.id,label:c.displayName})));
</script>
<InformationCard title={workspaceId?"Project agent models":"Workspace agent defaults"}>
  <p class="text-sm text-muted-foreground">{workspaceId?"Model overrides affect this project. Use workspace defaults to share model choices.":"These research and coding models are inherited by projects unless overridden."}</p>
  {#if workspaceId}<p class="mb-4 text-sm text-muted-foreground">{inherited?"Inherited from":"Overrides for"} the <a class="underline" href={`/workspaces/${encodeURIComponent(workspaceId)}`}>workspace defaults</a>.</p>{/if}
  <form method="POST" action="?/agentDefaults" use:enhance={submit} class="space-y-4">
    <input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="resourceVersion" value={project.resourceVersion} />
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="space-y-2"><Label for="project-agent-connection">Project agent connection</Label><ThemedSelect id="project-agent-connection" name="projectConnectionId" required value={projectConnection} onValueChange={value=>{projectConnection=value;projectModel="";}} options={options} placeholder="Choose connection" /><Label for="project-agent-model">Project agent model</Label><ThemedSelect id="project-agent-model" name="projectModel" required bind:value={projectModel} options={(catalogues[projectConnection]?.models??[]).map(m=>({value:m.id,label:m.id}))} placeholder="Choose available model" /></div>
      <div class="space-y-2"><Label for="coding-agent-connection">Coding agent connection</Label><ThemedSelect id="coding-agent-connection" name="codingConnectionId" required value={codingConnection} onValueChange={value=>{codingConnection=value;codingModel="";}} options={options} placeholder="Choose connection" /><Label for="coding-agent-model">Coding agent model</Label><ThemedSelect id="coding-agent-model" name="codingModel" required bind:value={codingModel} options={(catalogues[codingConnection]?.models??[]).map(m=>({value:m.id,label:m.id}))} placeholder="Choose available model" /></div>
    </div>
    {#if !connections.length}<p class="text-sm text-muted-foreground">Configure and test an <a href="/llm-providers/openai" class="underline">OpenAI connection</a> before selecting models.</p>{:else if (projectConnection&&!catalogues[projectConnection])||(codingConnection&&!catalogues[codingConnection])}<p role="status" class="text-sm text-muted-foreground">Models could not be loaded for the selected connection. Check connection access and reload.</p>{/if}
    <Button type="submit" disabled={pending||!projectConnection||!codingConnection||!projectModel||!codingModel}>{pending?"Saving…":"Save agent defaults"}</Button>
  </form>
  {#if workspaceId&&!inherited}<form method="POST" action="?/agentDefaults" use:enhance={submit} class="mt-3"><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}`} /><input type="hidden" name="resourceVersion" value={project.resourceVersion} /><input type="hidden" name="inherit" value="true" /><Button variant="outline" type="submit" disabled={pending}>Use workspace defaults</Button></form>{/if}
<details class="mt-4 space-y-3"><summary class="cursor-pointer font-medium">Agent capability profiles</summary><p class="text-sm text-muted-foreground">New executions freeze effective profiles. {workspaceId?"Project selections can narrow workspace tools, scope, budgets and skill revisions.":"Workspace selections stay within the application's planner and worker capabilities."}</p>
<form method="POST" action="?/agentProfiles" use:enhance={submit} class="space-y-3"><input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="resourceVersion" value={project.resourceVersion} />
<AgentProfileFields role="planner" configured={(workspaceId?configuredDefaults:project.agentDefaults)?.plannerProfile} inherited={project.agentDefaults?.plannerProfile} repositories={project.repositoryConnectionIds??[]} project={!!workspaceId} />
<AgentProfileFields role="worker" configured={(workspaceId?configuredDefaults:project.agentDefaults)?.workerProfile} inherited={project.agentDefaults?.workerProfile} repositories={project.repositoryConnectionIds??[]} project={!!workspaceId} />
<Button type="submit" disabled={pending}>{pending?"Saving…":"Save agent profiles"}</Button></form></details>
{#if error}<Feedback kind="error" title="Agent defaults not saved" message={error} />{:else if success}<p role="status" class="text-sm">{success}</p>{/if}
</InformationCard>
