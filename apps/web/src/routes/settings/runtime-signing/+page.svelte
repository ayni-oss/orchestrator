<script lang="ts">
 import { onMount } from 'svelte';
 import PageHeader from '$lib/components/page-header.svelte';
 import InformationCard from '$lib/components/information-card.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import { rpc, RpcError } from '$lib/api';
 type Signer = {id:string;displayName:string;keyId:string;publicKey:string;secretName:string;secretUid:string;activeRevision:string;revision:string;enabled:boolean};
 type Save = {idempotencyKey:string;id:string;expectedRevision:string;displayName:string;keyId:string;publicKey:string;secretName:string;secretUid:string};
 type Pending = {method:'SaveRuntimeSigner'|'SetRuntimeSignerEnabled';request:Save|{idempotencyKey:string;id:string;expectedRevision:string;enabled:boolean}};
 let signers=$state.raw<Signer[]>([]);let next=$state('');let busy=$state(true);let error=$state('');let notice=$state('');
 let selected=$state('');let selectedRevision=$state('0');let name=$state('');let keyId=$state('');let publicKey=$state('');let secretName=$state('');let secretUid=$state('');
 let pending=$state<Pending|null>(null);
 const storage='ayni.runtime.signer.pending';
 function persist(value:Pending|null){pending=value;if(value)sessionStorage.setItem(storage,JSON.stringify(value));else sessionStorage.removeItem(storage);}
 async function perform(action:()=>Promise<void>){busy=true;error='';try{await action();}catch(e){error=e instanceof RpcError&&e.status===409?'This signer or retained command has changed. Recover the original command, then refresh before making another update.':e instanceof RpcError&&e.status===404?'No receipt was found. Retry the original update if its response was interrupted.':e instanceof Error?e.message:'Signer configuration could not be saved.';}finally{busy=false;}}
 async function load(append=false){const result=await rpc('ListRuntimeSigners',{afterId:append?next:'',limit:50});signers=append?[...signers,...(result.signers||[])]:result.signers||[];next=result.nextAfterId||'';}
 function select(value:Signer|null){selected=value?.id||'';selectedRevision=value?.revision||'0';name=value?.displayName||'';keyId=value?.keyId||'';publicKey=value?.publicKey||'';secretName=value?.secretName||'';secretUid=value?.secretUid||'';notice='';error='';}
 async function accept(value:Signer){persist(null);await load();select(signers.find(s=>s.id===value.id)||null);notice=`${value.displayName} configuration saved. Signing access will be checked during preparation.`;}
 async function retry(){if(!pending)return;const result=await rpc(pending.method,pending.request);await accept(result.signer);}
 async function recover(){if(!pending)return;const result=await rpc('GetRuntimeSignerReceipt',{idempotencyKey:pending.request.idempotencyKey});await accept(result.signer);}
 async function save(){if(pending)return;persist({method:'SaveRuntimeSigner',request:{idempotencyKey:crypto.randomUUID(),id:selected,expectedRevision:selectedRevision,displayName:name,keyId,publicKey,secretName,secretUid}});await retry();}
 async function toggle(value:Signer){if(pending)return;persist({method:'SetRuntimeSignerEnabled',request:{idempotencyKey:crypto.randomUUID(),id:value.id,expectedRevision:value.revision,enabled:!value.enabled}});await retry();}
 onMount(()=>{void perform(async()=>{const saved=sessionStorage.getItem(storage);if(saved){const value=JSON.parse(saved);if(!['SaveRuntimeSigner','SetRuntimeSignerEnabled'].includes(value.method)||typeof value.request?.idempotencyKey!=='string')throw new Error('The saved signer command could not be read. Review retained configuration before starting another update.');pending=value;}await load();});});
</script>
<PageHeader title="Runtime signing" description="Register a public signing identity and an administrator-owned immutable Secret reference." />
<div class="mx-auto max-w-4xl space-y-6">
 {#if error}<Feedback kind="error" title="Signer needs attention" message={error}/>{/if}
 {#if notice}<Feedback title="Signer update" message={notice}/>{/if}
 {#if pending}<InformationCard title="Recover signer update"><p class="text-sm">The exact public configuration update is retained across reloads. Recover its receipt or retry the same update.</p><div class="flex flex-wrap gap-2"><Button disabled={busy} onclick={()=>perform(recover)}>Recover signer update</Button><Button variant="outline" disabled={busy} onclick={()=>perform(retry)}>Retry signer update</Button><Button variant="outline" disabled={busy} onclick={()=>persist(null)}>Dismiss signer recovery</Button></div></InformationCard>{/if}
 <InformationCard title={selected?'Update signer reference':'Add signer reference'}>
  <form class="space-y-4" onsubmit={event=>{event.preventDefault();void perform(save);}}>
   <div class="space-y-2"><Label for="signer-name">Name</Label><Input id="signer-name" bind:value={name} required maxlength={120} disabled={busy||!!pending}/></div>
   <div class="space-y-2"><Label for="signer-key-id">Key ID</Label><Input id="signer-key-id" bind:value={keyId} required maxlength={128} pattern={'[A-Za-z0-9_.\\-]+'} disabled={busy||!!pending||!!selected}/></div>
   <div class="space-y-2"><Label for="signer-public-key">Public key</Label><Input id="signer-public-key" bind:value={publicKey} required minlength={64} maxlength={64} pattern={'[0-9a-f]{64}'} disabled={busy||!!pending}/><p class="text-xs text-muted-foreground">Ed25519 public key as 64 lowercase hexadecimal characters. Review this identity against the committed repository trust policy.</p></div>
   <div class="space-y-2"><Label for="signer-secret-name">Immutable Secret name</Label><Input id="signer-secret-name" bind:value={secretName} required maxlength={63} pattern={'ayni-runtime-signer-[a-z0-9]([a-z0-9\\-]*[a-z0-9])?'} disabled={busy||!!pending}/></div>
   <div class="space-y-2"><Label for="signer-secret-uid">Secret UID</Label><Input id="signer-secret-uid" bind:value={secretUid} required maxlength={36} pattern={'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'} disabled={busy||!!pending}/></div>
   <p class="text-sm text-muted-foreground">An administrator provisions the immutable Secret in the deployment's signing namespace. Saving a reference does not validate Secret access or authorize a build.</p>
   <div class="flex flex-wrap gap-2"><Button type="submit" disabled={busy||!!pending}>Save signer reference</Button>{#if selected}<Button type="button" variant="outline" disabled={busy||!!pending} onclick={()=>select(null)}>Add another signer</Button>{/if}</div>
  </form>
 </InformationCard>
 <div class="flex items-center justify-between"><h2 class="text-lg font-semibold">Configured signers</h2><Button variant="outline" disabled={busy} onclick={()=>perform(()=>load())}>Refresh signers</Button></div>
 {#if busy}<p role="status" class="text-sm text-muted-foreground">Loading signer configuration…</p>{/if}
 {#if !signers.length&&!busy}<p class="text-sm text-muted-foreground">No signing references configured.</p>{/if}
 {#each signers as item(item.id)}<InformationCard title={item.displayName} status={item.enabled?'enabled':'disabled'}><InformationTable rows={[{label:'Key ID',value:item.keyId},{label:'Public key',value:item.publicKey,monospace:true},{label:'Secret name',value:item.secretName},{label:'Secret UID',value:item.secretUid,monospace:true},{label:'Configuration revision',value:item.revision}]}/><div class="flex flex-wrap gap-2"><Button variant="outline" disabled={busy||!!pending} onclick={()=>select(item)}>Edit signer reference</Button><Button variant="outline" disabled={busy||!!pending} onclick={()=>perform(()=>toggle(item))}>{item.enabled?'Disable signer':'Enable signer'}</Button></div></InformationCard>{/each}
 {#if next}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>load(true))}>More signers</Button>{/if}
</div>
