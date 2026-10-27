<script lang="ts">
 import { onMount, tick } from 'svelte';
 import { goto, invalidateAll } from '$app/navigation';
 import { Plus } from '@lucide/svelte';
 import { rpc } from '$lib/api';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import { Textarea } from '$lib/components/ui/textarea/index.js';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 let { workspaceId, label="New project" }: {workspaceId:string;label?:string}=$props();let open=$state(false);
 let name=$state('');let goal=$state('');let saving=$state(false);let error=$state('');
 let pending=$state<Record<string,string>|null>(null);
 const storageKey='ayni.project.pending';
 onMount(()=>{const saved=sessionStorage.getItem(storageKey);if(saved){try{pending=JSON.parse(saved);name=pending?.displayName||'';goal=pending?.goal||'';open=true;}catch{error='The pending command could not be restored.';}}});
 async function create(event:SubmitEvent){
  event.preventDefault();if(saving)return;saving=true;error='';
  try{
   if(!pending){pending={idempotencyKey:crypto.randomUUID(),workspaceId,displayName:name,goal};sessionStorage.setItem(storageKey,JSON.stringify(pending));}
   const result=await rpc('CreateProject',pending);
   pending=null;sessionStorage.removeItem(storageKey);open=false;await tick();
   await goto(`/?workspace=${encodeURIComponent(result.project.workspaceId)}`);await invalidateAll();
   name='';goal='';
  }catch(cause){error=cause instanceof Error?cause.message:'Project could not be created.';}finally{saving=false;}
 }
</script>
<Button onclick={()=>open=true}><Plus size={16}/>{label}</Button>
<Dialog.Root bind:open><Dialog.Content class="sm:max-w-xl"><Dialog.Header><Dialog.Title>Create project</Dialog.Title><Dialog.Description>Describe your goal.</Dialog.Description></Dialog.Header>
 <form onsubmit={create} class="space-y-5">
  <div class="space-y-2"><Label for="new-project-name">Project name</Label><Input id="new-project-name" bind:value={name} required maxlength={120} autocomplete="off" disabled={!!pending} /></div>
  <div class="space-y-2"><Label for="new-project-goal">Goal</Label><Textarea id="new-project-goal" bind:value={goal} required rows={4} disabled={!!pending} placeholder="Describe what this project should achieve and why." /></div>
  {#if pending}<p class="text-sm text-muted-foreground">A command is awaiting acknowledgement. Retry uses its original identity.</p>{/if}
  {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
  <Dialog.Footer><Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button><Button type="submit" disabled={saving}>{saving?'Creating…':pending?'Retry command':'Create project'}</Button></Dialog.Footer>
 </form>
</Dialog.Content></Dialog.Root>
