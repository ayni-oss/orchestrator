<script lang="ts">
 import { toast } from 'svelte-sonner';
 import { onMount, onDestroy } from 'svelte';
 import { invalidateAll } from '$app/navigation';
 import { protectUnsavedChanges } from '$lib/unsaved-changes.svelte';
 import { rpc } from '$lib/api';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import AutosaveField from '$lib/components/autosave-field.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 let { kind, id, name, resourceVersion, workspaceId, inline = false }: {
  kind:'workspace'|'project';id:string;name:string;resourceVersion:string;workspaceId?:string;inline?:boolean;
 }=$props();
 let open=$state(false);
 let draft=$state(''), saving=$state(false), error=$state('');
 let pending=$state<Record<string,string>|null>(null);
 let saveTimer: ReturnType<typeof setTimeout> | undefined;
 let savedRevision=$state(0);
 let ready=$state(false);
 const dirty=$derived(ready && (inline || open) && draft.trim()!==name);
 protectUnsavedChanges(()=>saving||(dirty&&!pending));
 onDestroy(()=>clearTimeout(saveTimer));
 function scheduleSave(value: string) {
  draft=value;
  error='';
  savedRevision=0;
  clearTimeout(saveTimer);
  if(value.trim()&&value.trim()!==name) saveTimer=setTimeout(()=>void save(),500);
 }
 const storageKey=$derived(`ayni.rename.${kind}.${id}`);
 onMount(()=>{
  if(inline)draft=name;
  ready=true;
  const saved=sessionStorage.getItem(storageKey);
  if(saved){try{
   const request=JSON.parse(saved);
   if(request.workspaceId!==(workspaceId??id)||(kind==='project'&&request.projectId!==id))throw new Error('scope');
   pending=request;draft=request.displayName;open=true;
  }catch{sessionStorage.removeItem(storageKey);}}
 });
 export function show(){if(!pending)draft=name;error='';open=true;}
 async function save(event?:SubmitEvent){
  event?.preventDefault();if(saving)return;saving=true;error='';
  try{
   if(!pending){
    pending={idempotencyKey:crypto.randomUUID(),workspaceId:workspaceId??id,expectedResourceVersion:resourceVersion,displayName:draft.trim(),...(kind==='project'?{projectId:id}:{})};
    sessionStorage.setItem(storageKey,JSON.stringify(pending));
   }
   await rpc(kind==='workspace'?'RenameWorkspace':'RenameProject',pending);
   sessionStorage.removeItem(storageKey);pending=null;open=false;await invalidateAll();
   if(inline){draft=name;savedRevision=Date.now();}
   else toast.success(kind==='workspace'?'Workspace renamed':'Project renamed');
  } catch(cause) {
   error=cause instanceof Error?cause.message:'The name could not be saved.';
   if(inline) toast.error('Could not save the name', { description: error });
  } finally {
   saving=false;
  }
 }
 async function discard(){
  sessionStorage.removeItem(storageKey);pending=null;open=false;error='';
  saving=true;
  try {
   await invalidateAll();
   if(inline)draft=name;
  ready=true;
  } catch(cause) {
   error=cause instanceof Error?cause.message:'The name could not be refreshed.';
   if(inline) toast.error('Could not refresh the name', { description: error });
  } finally {
   saving=false;
  }
 }
</script>
{#snippet renameForm()}
 <form onsubmit={save} class="space-y-5"><div class="space-y-2"><Label for={`rename-${kind}-${id}`}>Name</Label><Input id={`rename-${kind}-${id}`} bind:value={draft} required maxlength={120} disabled={!!pending||saving}/></div>
 {#if pending}<p class="text-sm text-muted-foreground">Retry uses the saved request. If the name changed elsewhere, close this request and refresh before renaming again.</p>{/if}
 {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
 <Dialog.Footer>{@render formActions()}</Dialog.Footer>
 </form>
{/snippet}
{#snippet formActions()}
 {#if pending}
  <Button type="button" variant="outline" disabled={saving} onclick={discard}>Close request and refresh</Button>
 {:else}
  <Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button>
 {/if}
 <Button type="submit" disabled={saving||!draft.trim()}>{saving?'Saving…':pending?'Retry rename':'Save name'}</Button>
{/snippet}
{#if inline}
 <div class="space-y-2">
  <Label for={`rename-${kind}-${id}`}>{kind === 'workspace' ? 'Workspace name' : 'Project Name'}</Label>
  <AutosaveField saving={saving ? !!pending : dirty && !!draft.trim() && !pending && !error} {savedRevision}>
   <Input data-slot="input-group-control" id={`rename-${kind}-${id}`} value={draft} oninput={event=>scheduleSave(event.currentTarget.value)} required maxlength={120} readonly={!!pending||saving} />
  </AutosaveField>
  {#if pending&&!saving}
   <div class="flex flex-wrap gap-2">
    <Button variant="outline" onclick={discard}>Close request and refresh</Button>
    <Button onclick={()=>save()}>Retry rename</Button>
   </div>
  {/if}
 </div>
{:else}
 <Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
   <Dialog.Header><Dialog.Title>Rename {kind}</Dialog.Title><Dialog.Description>Choose a name for this {kind}.</Dialog.Description></Dialog.Header>
   {@render renameForm()}
  </Dialog.Content>
 </Dialog.Root>
{/if}
