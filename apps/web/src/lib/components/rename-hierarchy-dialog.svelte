<script lang="ts">
 import { toast } from 'svelte-sonner';
 import { onMount } from 'svelte';
 import { invalidateAll } from '$app/navigation';
 import { rpc } from '$lib/api';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import InformationCard from '$lib/components/information-card.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 let { kind, id, name, resourceVersion, workspaceId, inline = false }: {
  kind:'workspace'|'project';id:string;name:string;resourceVersion:string;workspaceId?:string;inline?:boolean;
 }=$props();
 let open=$state(false);
 let draft=$state(''), saving=$state(false), error=$state('');
 let pending=$state<Record<string,string>|null>(null);
 const storageKey=$derived(`ayni.rename.${kind}.${id}`);
 onMount(()=>{
  if(inline)draft=name;
  const saved=sessionStorage.getItem(storageKey);
  if(saved){try{
   const request=JSON.parse(saved);
   if(request.workspaceId!==(workspaceId??id)||(kind==='project'&&request.projectId!==id))throw new Error('scope');
   pending=request;draft=request.displayName;open=true;
  }catch{sessionStorage.removeItem(storageKey);}}
 });
 export function show(){if(!pending)draft=name;error='';open=true;}
 async function save(event:SubmitEvent){
  event.preventDefault();if(saving)return;saving=true;error='';
  try{
   if(!pending){
    pending={idempotencyKey:crypto.randomUUID(),workspaceId:workspaceId??id,expectedResourceVersion:resourceVersion,displayName:draft.trim(),...(kind==='project'?{projectId:id}:{})};
    sessionStorage.setItem(storageKey,JSON.stringify(pending));
   }
   await rpc(kind==='workspace'?'RenameWorkspace':'RenameProject',pending);
   sessionStorage.removeItem(storageKey);pending=null;open=false;await invalidateAll();
   if(inline)draft=name;
   toast.success(kind==='workspace'?'Workspace renamed':'Project renamed');
  }catch(cause){error=cause instanceof Error?cause.message:'The name could not be saved.';}finally{saving=false;}
 }
 async function discard(){
  sessionStorage.removeItem(storageKey);pending=null;open=false;error='';
  saving=true;
  try {
   await invalidateAll();
   if(inline)draft=name;
  } catch(cause) {
   error=cause instanceof Error?cause.message:'The name could not be refreshed.';
  } finally {
   saving=false;
  }
 }
</script>
{#snippet renameForm()}
 <form onsubmit={save} class="space-y-5"><div class="space-y-2"><Label for={`rename-${kind}-${id}`}>{inline ? 'Workspace name' : 'Name'}</Label><Input id={`rename-${kind}-${id}`} bind:value={draft} required maxlength={120} disabled={!!pending||saving}/></div>
 {#if pending}<p class="text-sm text-muted-foreground">Retry uses the saved request. If the name changed elsewhere, close this request and refresh before renaming again.</p>{/if}
 {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
 {#if inline}
  <div class="flex flex-wrap gap-2">{@render formActions()}</div>
 {:else}
  <Dialog.Footer>{@render formActions()}</Dialog.Footer>
 {/if}
 </form>
{/snippet}
{#snippet formActions()}
 {#if pending}
  <Button type="button" variant="outline" disabled={saving} onclick={discard}>Close request and refresh</Button>
 {:else if !inline}
  <Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button>
 {/if}
 <Button type="submit" disabled={saving||!draft.trim()||(inline&&!pending&&draft.trim()===name)}>{saving?'Saving…':pending?'Retry rename':'Save name'}</Button>
{/snippet}
{#if inline}
 <InformationCard title="Rename workspace">{@render renameForm()}</InformationCard>
{:else}
 <Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
   <Dialog.Header><Dialog.Title>Rename {kind}</Dialog.Title><Dialog.Description>Choose a name for this {kind}.</Dialog.Description></Dialog.Header>
   {@render renameForm()}
  </Dialog.Content>
 </Dialog.Root>
{/if}
