<script lang="ts">
 import { onMount } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Textarea } from '$lib/components/ui/textarea/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import InformationCard from '$lib/components/information-card.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { rpc, type Connection } from '$lib/api';
 let { workspaceId }: { workspaceId: string }=$props();
 type Configuration={workspaceId:string;revisionId:string;connectionId:string;provider:string;model:string;instructions:string;maxInputTokens:string;maxOutputTokens:string;maxTotalTokens:string;idleTimeoutSeconds:string};
 let configuration=$state<Configuration|null>(null);let connections=$state.raw<Connection[]>([]);
 let connectionId=$state('');let model=$state('');let instructions=$state('');
 let input=$state('8192');let output=$state('2048');let total=$state('10240');let idle=$state('300');
 let busy=$state(true);let error=$state('');let notice=$state('');let pending=$state('');
 const storageKey=$derived(`ayni.workspace.${workspaceId}.configuration.pending`);
 const selected=$derived(connections.find(c=>c.id===connectionId));
 const models=$derived(selected?.models||[]);
 async function perform(action:()=>Promise<void>){busy=true;error='';try{await action();}catch(e){error=e instanceof Error?e.message:'Configuration unavailable.';}finally{busy=false;}}
 function apply(c:Configuration){configuration=c;connectionId=c.connectionId||'';model=c.model==='scripted'?'':c.model;instructions=c.instructions;input=c.maxInputTokens;output=c.maxOutputTokens;total=c.maxTotalTokens;idle=c.idleTimeoutSeconds;}
 async function load(){const result=await rpc('GetAgentConfiguration',{workspaceId,revisionId:''});apply(result.configuration);const values:Connection[]=[];let afterId='';let pages=0;do{pages++;const result=await rpc('ListConnections',{afterId,limit:100});values.push(...(result.connections||[]).filter((c:Connection)=>c.capability==='llm'));afterId=result.nextAfterId||'';}while(afterId&&pages<20);connections=values;}
 async function submit(event:SubmitEvent){event.preventDefault();if(!configuration||pending||!selected?.enabled||!selected.activeCredentialRevision)return;const request={idempotencyKey:crypto.randomUUID(),expectedRevisionId:configuration.revisionId,configuration:{workspaceId,revisionId:'',connectionId,provider:'openai',model,instructions,maxInputTokens:input,maxOutputTokens:output,maxTotalTokens:total,idleTimeoutSeconds:idle}};pending=JSON.stringify(request);sessionStorage.setItem(storageKey,pending);await recover();}
 async function recover(){await perform(async()=>{const result=await rpc('UpdateAgentConfiguration',JSON.parse(pending));pending='';sessionStorage.removeItem(storageKey);apply(result.configuration);notice='Workspace configuration saved. New turns use this connection and budget.';});}
 onMount(()=>{pending=sessionStorage.getItem(storageKey)||'';void perform(load);});
</script>
<div class="space-y-6">
 {#if error}<Feedback kind="error" title="Configuration needs attention" message={error}/>{/if}
 {#if notice}<Feedback title="Workspace updated" message={notice}/>{/if}
 {#if pending}<InformationCard title="Recover a configuration update"><p>Retry the retained command to recover its original result.</p><div class="flex gap-3"><Button disabled={busy} onclick={recover}>Recover update</Button><Button disabled={busy} variant="outline" onclick={()=>{pending='';sessionStorage.removeItem(storageKey);}}>Abandon recovery</Button></div></InformationCard>{/if}
 {#if configuration}<InformationCard title="Agent configuration"><form onsubmit={submit} class="space-y-5">
  <div class="space-y-2"><Label for="workspace-connection">OpenAI connection</Label><select id="workspace-connection" class="h-10 w-full border border-input bg-background px-3 text-sm" bind:value={connectionId} onchange={()=>model=''} required disabled={busy||!!pending}><option value="" disabled>Select a connection</option>{#each connections as c (c.id)}<option value={c.id} disabled={!c.enabled||!c.activeCredentialRevision}>{c.displayName}{!c.enabled?' (disabled)':!c.activeCredentialRevision?' (validation required)':''}</option>{/each}</select><p class="text-xs text-muted-foreground"><a class="underline" href="/settings/llm-providers/openai">Manage OpenAI connections</a></p></div>
  <div class="space-y-2"><Label for="workspace-model">Model</Label><select id="workspace-model" class="h-10 w-full border border-input bg-background px-3 text-sm" bind:value={model} required disabled={busy||!!pending||!selected}><option value="" disabled>Select a discovered model</option>{#each models as id (id)}<option value={id}>{id}</option>{/each}</select></div>
  <div class="space-y-2"><Label for="workspace-instructions">Instructions</Label><Textarea id="workspace-instructions" bind:value={instructions} maxlength={16384} rows={5} disabled={busy||!!pending}/></div>
  <div class="grid gap-4 sm:grid-cols-2">
   <div class="space-y-2"><Label for="max-input">Input token budget</Label><Input id="max-input" type="number" min={1} max={1000000} value={input} oninput={event=>input=event.currentTarget.value} required disabled={busy||!!pending}/></div>
   <div class="space-y-2"><Label for="max-output">Output token budget</Label><Input id="max-output" type="number" min={1} max={100000} value={output} oninput={event=>output=event.currentTarget.value} required disabled={busy||!!pending}/></div>
   <div class="space-y-2"><Label for="max-total">Total token budget</Label><Input id="max-total" type="number" min={1} max={1100000} value={total} oninput={event=>total=event.currentTarget.value} required disabled={busy||!!pending}/></div>
   <div class="space-y-2"><Label for="idle-timeout">Idle timeout (seconds)</Label><Input id="idle-timeout" type="number" min={30} max={86400} value={idle} oninput={event=>idle=event.currentTarget.value} required disabled={busy||!!pending}/></div>
  </div>
  <Button type="submit" disabled={busy||!!pending||!selected?.enabled||!selected?.activeCredentialRevision||!models.includes(model)}>Save workspace configuration</Button>
 </form></InformationCard>{/if}
 <Button variant="outline" disabled={busy} onclick={()=>perform(load)}>Refresh configuration</Button>
</div>
