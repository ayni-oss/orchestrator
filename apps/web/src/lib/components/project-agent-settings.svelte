<script lang="ts">
  import { untrack } from "svelte";
  import type { PageData } from "../../routes/projects/[projectId]/$types";
  import { enhance } from "$lib/forms.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  let {project,connections,catalogues,idempotencyKey}:{project:NonNullable<PageData["project"]>;connections:PageData["llmConnections"];catalogues:PageData["connectionModels"];idempotencyKey:string}=$props();
  let projectConnection=$state(untrack(()=>project.agentDefaults?.projectConnectionId??"")),codingConnection=$state(untrack(()=>project.agentDefaults?.codingConnectionId??""));
  let projectModel=$state(untrack(()=>project.agentDefaults?.projectModel??"")),codingModel=$state(untrack(()=>project.agentDefaults?.codingModel??""));
  let pending=$state(false);
  const options=$derived(connections.map(c=>({value:c.id,label:c.displayName})));
</script>
<InformationCard title="Agent models">
  <form method="POST" action="?/agentDefaults" use:enhance={()=>{pending=true;return async({update})=>{try{await update({reset:false});}finally{pending=false;}};}} class="space-y-4">
    <input type="hidden" name="idempotencyKey" value={idempotencyKey} /><input type="hidden" name="resourceVersion" value={project.resourceVersion} />
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="space-y-2"><Label for="project-agent-connection">Project agent connection</Label><ThemedSelect id="project-agent-connection" name="projectConnectionId" required value={projectConnection} onValueChange={value=>{projectConnection=value;projectModel="";}} options={options} placeholder="Choose connection" /><Label for="project-agent-model">Project agent model</Label><ThemedSelect id="project-agent-model" name="projectModel" required bind:value={projectModel} options={(catalogues[projectConnection]?.models??[]).map(m=>({value:m.id,label:m.id}))} placeholder="Choose available model" /></div>
      <div class="space-y-2"><Label for="coding-agent-connection">Coding agent connection</Label><ThemedSelect id="coding-agent-connection" name="codingConnectionId" required value={codingConnection} onValueChange={value=>{codingConnection=value;codingModel="";}} options={options} placeholder="Choose connection" /><Label for="coding-agent-model">Coding agent model</Label><ThemedSelect id="coding-agent-model" name="codingModel" required bind:value={codingModel} options={(catalogues[codingConnection]?.models??[]).map(m=>({value:m.id,label:m.id}))} placeholder="Choose available model" /></div>
    </div>
    {#if !connections.length}<p class="text-sm text-muted-foreground">Configure and test an <a href="/llm-providers/openai" class="underline">OpenAI connection</a> before selecting models.</p>{:else if (projectConnection&&!catalogues[projectConnection])||(codingConnection&&!catalogues[codingConnection])}<p role="status" class="text-sm text-muted-foreground">Models could not be loaded for the selected connection. Check connection access and reload.</p>{/if}
    <Button type="submit" disabled={pending||!projectConnection||!codingConnection||!projectModel||!codingModel}>Save agent defaults</Button>
  </form>
</InformationCard>
