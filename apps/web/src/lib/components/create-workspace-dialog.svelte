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
 let { open=$bindable(false), showTrigger=false }: {open?:boolean;showTrigger?:boolean}=$props();
 let name=$state('');let goal=$state('');let saving=$state(false);let error=$state('');
 let pending=$state<Record<string,string>|null>(null);
 const storageKey='ayni.workspace.pending';
 onMount(()=>{const saved=sessionStorage.getItem(storageKey);if(saved){try{pending=JSON.parse(saved);name=pending?.displayName||'';goal=pending?.goal||'';open=true;}catch{error='The pending command could not be restored.';}}});
 async function create(event:SubmitEvent){
  event.preventDefault();if(saving)return;saving=true;error='';
  try{
   if(!pending){pending={idempotencyKey:crypto.randomUUID(),displayName:name};sessionStorage.setItem(storageKey,JSON.stringify(pending));}
   const result=await rpc('CreateWorkspace',pending);
   pending=null;sessionStorage.removeItem(storageKey);open=false;await tick();
   await goto(`/?workspace=${encodeURIComponent(result.workspace.id)}`);await invalidateAll();
   name='';goal='';
  }catch(cause){error=cause instanceof Error?cause.message:'Workspace could not be created.';}finally{saving=false;}
 }
</script>
{#if showTrigger}<Button onclick={()=>open=true}><Plus size={16}/>Create workspace</Button>{/if}
<Dialog.Root bind:open><Dialog.Content class="sm:max-w-xl"><Dialog.Header><Dialog.Title>Create workspace</Dialog.Title><Dialog.Description>Create a workspace for your projects.</Dialog.Description></Dialog.Header>
 <form onsubmit={create} class="space-y-5">
  <div class="space-y-2"><Label for="new-workspace-name">Workspace name</Label><Input id="new-workspace-name" bind:value={name} required maxlength={120} autocomplete="off" disabled={!!pending} /></div>

  {#if pending}<p class="text-sm text-muted-foreground">A command is awaiting acknowledgement. Retry uses its original identity.</p>{/if}
  {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
  <Dialog.Footer><Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button><Button type="submit" disabled={saving}>{saving?'Creating…':pending?'Retry command':'Create workspace'}</Button></Dialog.Footer>
 </form>
</Dialog.Content></Dialog.Root>
