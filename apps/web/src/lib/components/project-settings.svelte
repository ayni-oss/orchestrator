<script lang="ts">
  import {untrack} from "svelte";
  import type {PageData} from "../../routes/projects/[projectId]/$types";
  import {enhance} from "$lib/forms.js";
  import InformationCard from "$lib/components/information-card.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import ProjectAgentSettings from "$lib/components/project-agent-settings.svelte";
  import ConversationSettings from "$lib/components/conversation-settings.svelte";
  import AyniIcon from "$lib/components/ayni-icon.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import {Input} from "$lib/components/ui/input/index.js";
  import {Textarea} from "$lib/components/ui/textarea/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  let {data}:{data:PageData}=$props();
  let saving=$state(false),settingsError=$state(""),settingsSuccess=$state("");
  let projectName=$state(untrack(()=>data.project?.displayName??"")),projectGoal=$state(untrack(()=>data.project?.goal??""));
  let members=$state(untrack(()=>[...(data.project?.repositoryConnectionIds??[])]));
  const selectedMembers=$derived(members);
</script>
<section class="space-y-4" aria-label="Project settings">
  <h2 class="text-lg font-semibold">Settings</h2>
  {#if data.project?.state==="active"&&data.workspace?.state==="active"}
    <InformationCard title="Project details">
          <form use:enhance={() => {saving=true;settingsError="";settingsSuccess="";return async ({ result, update }) => {try{if(result.type === "failure")settingsError=String(result.data?.error?.message??"Settings could not be saved. Try again.");if(result.type === "success"){settingsSuccess=String(result.data?.success??"Project updated.");}await update({reset:false});}finally{saving=false;}};}} method="POST" action="?/update" class="space-y-3">
            <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} />
            <input type="hidden" name="workspaceId" value={data.project.workspaceId} />
            <Label for="goal">Goal and planning instructions</Label><Textarea id="goal" name="goal" bind:value={projectGoal} required maxlength={16384} rows={4} /><p class="text-xs text-muted-foreground">Describe the outcome and project constraints. Repository instructions such as AGENTS.md remain in each repository.</p>
            <Label for="name">Project name</Label><Input id="name" name="displayName" bind:value={projectName} required maxlength={256} />
            <Label for="members">Repositories</Label><RepositoryPicker id="members" repositories={data.choices} value={selectedMembers} onchange={value => members=value} />
            <p class="text-xs text-muted-foreground">Removing a repository prevents new tasks and retries for it; task history is retained.</p>
            {#if settingsError}<Feedback kind="error" title="Settings not saved" message={settingsError} />{/if}
            <Button type="submit" disabled={saving}>{saving?"Saving…":"Save settings"}</Button>
            {#if settingsSuccess}<p role="status">{settingsSuccess}</p>{/if}
          </form>
    </InformationCard>
          <section class="space-y-3" aria-label="Project repositories"><h2 class="font-semibold">Selected repositories</h2><p class="text-sm text-muted-foreground">Open a repository to inspect its shared runtimes.</p>
            {#each data.repositories as repository (repository.id)}<a class="flex items-center gap-2 break-all text-sm underline" href={`/repositories/${encodeURIComponent(repository.id)}`}>{repository.displayName}{#if repository.useAyni !== false}<AyniIcon class="size-4 shrink-0" role="img" aria-label="Ayni enabled" />{/if}</a>{/each}
          </section>
          {#if data.workspace?.state === "active"}{#key data.project.id}<ProjectAgentSettings capabilities={data.capabilities} configuration={data.agentConfiguration} project={data.project} inherited={!data.project.agentDefaults?.projectConnectionId&&!data.project.agentDefaults?.codingConnectionId} workspaceId={data.workspace.id} connections={data.llmConnections} catalogues={data.connectionModels} idempotencyKey={data.idempotencyKey} />{/key}{/if}
  {:else}<p class="text-sm text-muted-foreground">This project's settings are read only because the project or workspace is archived.</p>{/if}
  {#if data.conversation}{#key data.conversation.id}<ConversationSettings {data} />{/key}{/if}
</section>
