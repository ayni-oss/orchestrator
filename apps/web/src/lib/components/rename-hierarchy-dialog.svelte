<script lang="ts">
 import { toast } from 'svelte-sonner';
 import { onMount } from 'svelte';
 import { invalidateAll } from '$app/navigation';
 import { rpc } from '$lib/api';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 let { kind, id, name, resourceVersion, workspaceId, open=$bindable(false) }: {
  kind:'workspace'|'project';id:string;name:string;resourceVersion:string;workspaceId?:string;open?:boolean;
 }=$props();
 let draft=$state(''), saving=$state(false), error=$state('');
 let pending=$state<Record<string,string>|null>(null);
 const storageKey=$derived(`ayni.rename.${kind}.${id}`);
 onMount(()=>{
  if(open)draft=name;
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
   toast.success(kind==='workspace'?'Workspace renamed':'Project renamed');
  }catch(cause){error=cause instanceof Error?cause.message:'The name could not be saved.';}finally{saving=false;}
 }
 function discard(){sessionStorage.removeItem(storageKey);pending=null;open=false;void invalidateAll();}
</script>
<Dialog.Root bind:open><Dialog.Content class="sm:max-w-lg"><Dialog.Header><Dialog.Title>Rename {kind}</Dialog.Title><Dialog.Description>Choose a name for this {kind}.</Dialog.Description></Dialog.Header>
 <form onsubmit={save} class="space-y-5"><div class="space-y-2"><Label for={`rename-${kind}-${id}`}>Name</Label><Input id={`rename-${kind}-${id}`} bind:value={draft} required maxlength={120} disabled={!!pending||saving}/></div>
 {#if pending}<p class="text-sm text-muted-foreground">Retry uses the saved request. If the name changed elsewhere, close this request and refresh before renaming again.</p>{/if}
 {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
 <Dialog.Footer>{#if pending}<Button type="button" variant="outline" disabled={saving} onclick={discard}>Close request and refresh</Button>{:else}<Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button>{/if}<Button type="submit" disabled={saving||!draft.trim()}>{saving?'Saving…':pending?'Retry rename':'Save name'}</Button></Dialog.Footer></form>
</Dialog.Content></Dialog.Root>
