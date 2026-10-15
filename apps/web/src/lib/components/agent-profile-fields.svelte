<script lang="ts">
  import {untrack} from "svelte";
  import {type ProfileRole} from "$lib/agent-profiles.js";
  import {Input} from "$lib/components/ui/input/index.js";
  import {Textarea} from "$lib/components/ui/textarea/index.js";
  import {Label} from "$lib/components/ui/label/index.js";
  import {Button} from "$lib/components/ui/button/index.js";
  import type {AgentCapabilitiesDto,AgentProfileSettingsDto} from "@ayni-orchestrator/bff/server";
  let {role,configured,inherited,effective,capabilities,repositories,project=false}:{role:ProfileRole;configured?:AgentProfileSettingsDto|undefined;inherited?:AgentProfileSettingsDto|undefined;effective?:AgentProfileSettingsDto|undefined;capabilities:AgentCapabilitiesDto;repositories:string[];project?:boolean}=$props();
  const defaults=$derived(capabilities.roles[role]);
  const limits=$derived(capabilities.limits);
  let useInherited=$state(untrack(()=>!configured));
  const initial=untrack(()=>configured??inherited??capabilities.roles[role]);
  let skills=$state(initial.skills.map((s,index)=>({...s,key:index})));
  let nextKey=untrack(()=>skills.length);
</script>
<fieldset class="space-y-3 rounded-lg border p-4">
  <legend class="px-2 font-medium">{role==="planner"?"Planner profile":"Worker profile"}</legend>
  <label class="flex items-center gap-2 text-sm"><input type="checkbox" name={`${role}.inherit`} value="true" bind:checked={useInherited} />{project?"Use workspace capabilities":"Use application capabilities"}</label>
  <p class="text-xs text-muted-foreground">{role==="planner"?"Research tools can read selected repositories and prepare proposals. They cannot edit repositories or execute commands.":"Worker tools operate inside the approved task and sandbox. The worker always performs its mandatory final quality check."}</p>
  {#if useInherited}<p class="break-words text-xs text-muted-foreground">Inherited tools: {(inherited??defaults).allowedTools.join(", ")} · Up to {(inherited??defaults).maxModelCalls} model calls · {(inherited??defaults).deadlineSeconds} seconds</p>{/if}
  {#if effective}<details class="text-xs"><summary>Effective settings for new executions</summary><p>Tools: {effective.allowedTools.join(", ")} · {effective.maxModelCalls} calls · {effective.deadlineSeconds} seconds</p><p class="whitespace-pre-wrap">{effective.instructions||"Application role instructions"}</p></details>{/if}
  <div class={useInherited?"hidden":"space-y-3"}>
    <Label for={`${role}-instructions`}>{project?"Additional instructions":"Instructions"}</Label><Textarea id={`${role}-instructions`} name={`${role}.instructions`} value={configured?.instructions??""} maxlength={limits.maxInstructionBytes} rows={3} />
    <p class="text-xs text-muted-foreground">Instructions and skills are model context. They cannot add permissions. Do not include credential values.</p>
    <div class="grid gap-3 sm:grid-cols-2"><div><Label for={`${role}-calls`}>Maximum model calls</Label><Input id={`${role}-calls`} name={`${role}.calls`} type="number" min={1} max={limits.maxModelCalls} value={initial.maxModelCalls} /></div><div><Label for={`${role}-deadline`}>Maximum duration (seconds)</Label><Input id={`${role}-deadline`} name={`${role}.deadline`} type="number" min={1} max={Number(limits.maxDeadlineSeconds)} value={initial.deadlineSeconds} /></div></div>
    <fieldset class="space-y-1"><legend class="text-sm font-medium">Allowed tools</legend>{#each defaults.allowedTools as tool (tool)}<label class="flex items-center gap-2 text-sm"><input type="checkbox" name={`${role}.tools`} value={tool} checked={initial.allowedTools.includes(tool)} />{tool}</label>{/each}</fieldset>
    <fieldset class="space-y-1"><legend class="text-sm font-medium">Repository scope</legend><p class="text-xs text-muted-foreground">Leave unselected to use current project membership. Project selections must stay within workspace scope.</p>{#each repositories as id (id)}<label class="flex items-center gap-2 break-all text-sm"><input type="checkbox" name={`${role}.repositories`} value={id} checked={initial.repositoryIds.includes(id)} />{id}</label>{/each}</fieldset>
    <div class="space-y-3"><h4 class="text-sm font-medium">Pinned skills</h4><p class="text-xs text-muted-foreground">A skill's revision and text are frozen together. Project skills must match workspace revisions.</p>
    {#each skills as skill,index (skill.key)}<fieldset class="space-y-2 rounded border p-3"><legend class="text-xs">Skill {index+1}</legend><Label for={`${role}-skill-${skill.key}-id`}>Skill ID</Label><Input id={`${role}-skill-${skill.key}-id`} name={`${role}.skill.${index}.id`} bind:value={skill.id} maxlength={limits.maxIdentifierBytes} /><Label for={`${role}-skill-${skill.key}-revision`}>Revision</Label><Input id={`${role}-skill-${skill.key}-revision`} name={`${role}.skill.${index}.revision`} bind:value={skill.revision} maxlength={limits.maxIdentifierBytes} /><Label for={`${role}-skill-${skill.key}-text`}>Skill instructions</Label><Textarea id={`${role}-skill-${skill.key}-text`} name={`${role}.skill.${index}.instructions`} bind:value={skill.instructions} maxlength={limits.maxInstructionBytes} rows={2} /><Button type="button" variant="outline" onclick={()=>skills=skills.filter(s=>s.key!==skill.key)}>Remove skill {index+1}</Button></fieldset>{/each}
    {#if skills.length<limits.maxSkills}<Button type="button" variant="outline" onclick={()=>skills=[...skills,{id:"",revision:"",instructions:"",key:nextKey++}]}>Add pinned skill</Button>{/if}
    </div>
  </div>
</fieldset>
