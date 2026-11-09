<script lang="ts">
 import { toast } from 'svelte-sonner';
 import {onMount} from 'svelte';
 import InformationCard from '$lib/components/information-card.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import {Button} from '$lib/components/ui/button/index.js';
 import {Input} from '$lib/components/ui/input/index.js';
 import {Label} from '$lib/components/ui/label/index.js';
 import {rpc,RpcError} from '$lib/api';
 import {generateSigningSeed,provisionIntent,provisioningReasons,signingSeed,trustSnippet,validateSignerFields,type RuntimeSigner as Signer,type ProvisionIntent,type ProvisionOperation,type ProvisionResult} from '$lib/runtime-signing';
 let { createOnMount = false, onSaved }: { createOnMount?: boolean; onSaved?: (signer: Signer) => void } = $props();
 type Save={idempotencyKey:string;id:string;expectedRevision:string;displayName:string;keyId:string;publicKey:string;secretName:string;secretUid:string};
 type Pending={method:'SaveRuntimeSigner'|'SetRuntimeSignerEnabled';request:Save|{idempotencyKey:string;id:string;expectedRevision:string;enabled:boolean}};
 let editorOpen=$state(false);
 let signers=$state.raw<Signer[]>([]);let next=$state('');let busy=$state(true);let error=$state('');
 let selected=$state('');let selectedRevision=$state('0');let name=$state('');let keyId=$state('');let publicKey=$state('');let secretName=$state('');let secretUid=$state('');
 type SignerMode = 'generate'|'backup'|'import'|'manual';
 let backupDownloaded=$state(false);
 let mode=$state<SignerMode>('generate');let seed=$state('');let seedFile=$state<HTMLInputElement|null>(null);let privateReadGeneration=0;
 let pending=$state<Pending|null>(null);let setup=$state<ProvisionIntent|null>(null);let operation=$state.raw<ProvisionOperation|null>(null);let receiptMissing=$state(false);let inputRejected=$state(false);
 const storage='ayni.runtime.signer.pending';const setupStorage='ayni.runtime.signer.provisioning.pending';
 const hasPending=$derived(!!pending||!!setup);
 const canCorrect=$derived(!!setup&&inputRejected&&receiptMissing);
 const canResume=$derived(!!setup&&(operation?.state==='awaiting_material'||receiptMissing));
 function clearPrivate(){privateReadGeneration++;seed='';backupDownloaded=false;if(seedFile)seedFile.value='';}
 function persist(value:Pending|null){pending=value;if(value)sessionStorage.setItem(storage,JSON.stringify(value));else sessionStorage.removeItem(storage);}
 function retainSetup(value:ProvisionIntent|null){setup=value;if(value)sessionStorage.setItem(setupStorage,JSON.stringify(value));else sessionStorage.removeItem(setupStorage);}
 async function perform(action:()=>Promise<void>){busy=true;error='';try{await action();}catch(e){error=e instanceof RpcError&&e.status===409?'This signer or retained command has changed. Recover the original command, then refresh before making another update.':e instanceof RpcError&&e.status===404?'No receipt was found. Retry the original update if its response was interrupted.':'Signer configuration could not be loaded or saved. Recover a retained command before retrying.';}finally{busy=false;}}
 async function load(append=false){const result=await rpc('ListRuntimeSigners',{afterId:append?next:'',limit:50});signers=append?[...signers,...(result.signers||[])]:result.signers||[];next=result.nextAfterId||'';}
 function select(value:Signer|null){editorOpen=true;clearPrivate();selected=value?.id||'';selectedRevision=value?.revision||'0';name=value?.keyId||'';keyId=value?.keyId||'';publicKey=value?.publicKey||'';secretName=value?.secretName||'';secretUid=value?.secretUid||'';mode=value?'manual':'generate';error='';}
 function changeMode(value:SignerMode){clearPrivate();mode=value;error='';}
 function closeEditor(){select(null);editorOpen=false;}
 async function accept(value:Signer){
  // A completed public response is enough to display the identity safely even
  // when the following list refresh is unavailable. It never enables a second
  // generate submission: another key requires an explicit Add signing key action.
  signers=[value,...signers.filter(signer=>signer.id!==value.id)];select(value);persist(null);
  try{await load();select(signers.find(signer=>signer.id===value.id)||value);}catch{toast.warning('The signing identity was saved. Refresh the list when the service is available.');}
  editorOpen=false;
 }
 async function retry(){if(!pending)return;const result=await rpc(pending.method,pending.request);await accept(result.signer);}
 async function recover(){if(!pending)return;const result=await rpc('GetRuntimeSignerReceipt',{idempotencyKey:pending.request.idempotencyKey});await accept(result.signer);}
 async function save(){if(hasPending)return;keyId=name;validateSignerFields(name,keyId);persist({method:'SaveRuntimeSigner',request:{idempotencyKey:crypto.randomUUID(),id:selected,expectedRevision:selectedRevision,displayName:name,keyId,publicKey,secretName,secretUid}});await retry();}
 async function toggle(value:Signer){if(hasPending)return;persist({method:'SetRuntimeSignerEnabled',request:{idempotencyKey:crypto.randomUUID(),id:value.id,expectedRevision:value.revision,enabled:!value.enabled}});await retry();}
 function setupError(cause:unknown){
  if(cause instanceof RpcError){
   if(provisioningReasons[cause.message])return provisioningReasons[cause.message];
   if(cause.status===503)return 'The signer setup service is unavailable. Recover this request when access is restored; ask the administrator to check the deployment.';
   if(cause.status===409)return provisioningReasons.signer_provisioning_conflict;
   if(cause.status===400)return 'The signing input was rejected. Check the key name and raw seed format, then recover the original setup before retrying.';
  }
  return 'The setup response was interrupted. Recover this exact setup before starting another; no private material is retained in this browser.';
 }
 async function acceptSetup(result:ProvisionResult){
  const intent=setup;if(!intent)return;
  if(result.operation.idempotencyKey!==intent.idempotencyKey||result.operation.mode!==intent.mode||result.operation.keyId!==intent.keyId)throw new Error('setup identity mismatch');
  operation=result.operation;receiptMissing=false;
  if(result.signer&&result.operation.state==='completed'){await accept(result.signer);retainSetup(null);operation=null;onSaved?.(result.signer);toast.success(`Signing identity ${result.signer.displayName} configured`, { description: 'Merge its public entry into the repository trust policy, commit it and regenerate the lock before inspecting again.' });}
 }
 async function submitSetup(){
  let material='';let request:(ProvisionIntent&{signingSeed:string})|null=null;busy=true;error='';
  try{
   if(mode==='backup'&&!backupDownloaded)return;
   if(!setup||canCorrect){keyId=name;validateSignerFields(name,keyId);}
   const intent:ProvisionIntent=setup?(canCorrect?{...setup,displayName:name,keyId}:setup):{idempotencyKey:crypto.randomUUID(),displayName:name,keyId,mode:mode==='generate'?'generate':'import'};
   if(intent.mode==='import')material=signingSeed(seed);
   if(!setup||canCorrect)retainSetup(intent);
   inputRejected=false;receiptMissing=false;
   clearPrivate();
   mode=intent.mode;
   request={...intent,signingSeed:material};
   await acceptSetup(await rpc('ProvisionRuntimeSigner',request));
  }catch(cause){if(cause instanceof RpcError&&cause.status===400)inputRejected=true;error=cause instanceof Error&&!(cause instanceof RpcError)&&(!setup||canCorrect)?cause.message:setupError(cause);}
  finally{if(request)request.signingSeed='';request=null;material='';clearPrivate();busy=false;}
 }
 async function recoverSetup(){
  if(!setup)return;busy=true;error='';clearPrivate();
  try{await acceptSetup(await rpc('RecoverRuntimeSignerProvisioning',{idempotencyKey:setup.idempotencyKey}));}
  catch(cause){if(cause instanceof RpcError&&cause.status===404){receiptMissing=true;operation=null;toast.warning(inputRejected?'The rejected setup has no receipt. Correct its key name, then resume with the same recovery key.':'No setup receipt was found. Resume the same request below with the same key name and recovery key.');}else error=setupError(cause);}
  finally{clearPrivate();busy=false;}
 }
 async function downloadBackup() {
  if (busy || hasPending || mode !== 'backup' || backupDownloaded) return;
  const generation = privateReadGeneration;
  busy = true;
  error = '';
  try {
   validateSignerFields(name, name);
   const generated = await generateSigningSeed();
   if (generation !== privateReadGeneration) return;
   const url = URL.createObjectURL(new Blob([generated + '\n'], { type: 'application/octet-stream' }));
   try {
    const link = document.createElement('a');
    link.href = url;
    link.download = name + '.ed25519-seed';
    link.click();
   } finally {
    setTimeout(() => URL.revokeObjectURL(url), 0);
   }
   seed = generated;
   backupDownloaded = true;
  } catch {
   error = 'Could not generate a backup. Check the key name or choose server generation or import.';
  } finally {
   busy = false;
  }
 }
 async function copyPublicKey(value: Signer) {
  try {
   await navigator.clipboard.writeText(value.publicKey);
   toast.success('Public key copied');
  } catch {
   error = 'Select and copy the displayed public key.';
  }
 }
 async function readSeedFile(event:Event){
  const input=event.currentTarget as HTMLInputElement;const file=input.files?.[0];clearPrivate();if(!file)return;
  const generation=privateReadGeneration;
  try{if(file.size<64||file.size>65)throw new Error('Signing seed files must contain 64 lowercase hexadecimal characters, with at most one final newline.');const value=new TextDecoder('utf-8',{fatal:true}).decode(await file.arrayBuffer());const normalized=signingSeed(value);if(generation===privateReadGeneration)seed=normalized;}
  catch{if(generation===privateReadGeneration){clearPrivate();error='Choose a raw signing seed file containing exactly 64 lowercase hexadecimal characters, with at most one final newline.';}}
 }
 async function copyTrust(value:Signer){try{await navigator.clipboard.writeText(trustSnippet(value));toast.success('Public trust policy copied', { description: 'Commit it and regenerate the repository lock before inspecting again.' });}catch{error='Copy the displayed public trust policy into the repository policy.';}}
 onMount(()=>{void perform(async()=>{
  const saved=sessionStorage.getItem(storage);if(saved){const value=JSON.parse(saved);if(!['SaveRuntimeSigner','SetRuntimeSignerEnabled'].includes(value.method)||typeof value.request?.idempotencyKey!=='string')throw new Error('invalid public command');pending=value;}
  const savedSetup=sessionStorage.getItem(setupStorage);if(savedSetup){const value=provisionIntent(JSON.parse(savedSetup));if(!value)throw new Error('invalid public setup');setup=value;name=value.keyId;keyId=value.keyId;mode=value.mode;}
  editorOpen=!!pending||!!setup||createOnMount;await load();if(setup)await recoverSetup();
 });return clearPrivate;});
</script>
<div class="mx-auto max-w-4xl space-y-6">
 <div class="flex flex-wrap items-center justify-between gap-3"><h2 class="text-lg font-semibold">Signing keys</h2><Button disabled={busy||hasPending} onclick={()=>select(null)}>Add signing key</Button></div>
 {#if error}<Feedback kind="error" title="Signer needs attention" message={error}/>{/if}
 {#if pending}<InformationCard title="Recover signer update"><p class="text-sm">The exact public reference update is retained across reloads.</p><div class="flex flex-wrap gap-2"><Button disabled={busy} onclick={()=>perform(recover)}>Recover signer update</Button><Button variant="outline" disabled={busy} onclick={()=>perform(retry)}>Retry signer update</Button><Button variant="outline" disabled={busy} onclick={()=>persist(null)}>Dismiss signer recovery</Button></div></InformationCard>{/if}
 {#if setup}<InformationCard title="Recover signer setup" status={operation?.state||'response unknown'}>
  <p class="text-sm">{setup.displayName} · Key ID {setup.keyId} · {setup.mode==='generate'?'Generate a key':'Import a key'}. Recovery continues this exact setup.</p>
  {#if operation?.safeReason}<p class="text-sm">{provisioningReasons[operation.safeReason]||'The setup needs administrator attention. Recover it after the deployment is corrected.'}</p>{/if}
  <p class="text-sm text-muted-foreground">Private material is cleared after each submission. Recovery never creates a second signing identity.</p>
  <Button disabled={busy} onclick={recoverSetup}>Recover signer setup</Button>
  {#if canResume&&setup.mode==='generate'}<Button variant="outline" disabled={busy} onclick={submitSetup}>Resume signer setup</Button>{/if}
 </InformationCard>{/if}
 {#if editorOpen||setup}
 <InformationCard title={selected?'Update signer reference':setup?'Resume signer setup':'Add signing key'}>
  <form class="space-y-4" onsubmit={event=>{event.preventDefault();if(mode==='manual')void perform(save);else void submitSetup();}}>
   <div class="space-y-2"><Label for="signer-name">Name</Label><Input id="signer-name" bind:value={name} required maxlength={120} disabled={busy||backupDownloaded||!!pending||(!!setup&&!canCorrect)}/><p class="text-xs text-muted-foreground">Use this exact name in .ayni.toml with the public key. Use letters, numbers, dots, underscores or hyphens.</p></div>
   {#if !setup}<fieldset disabled={busy||!!pending||!!selected} class="space-y-2"><legend class="text-sm font-medium">Signing key</legend><div class="flex flex-wrap gap-4 text-sm">
    <label class="flex items-center gap-2"><input type="radio" name="signer-mode" value="generate" checked={mode==='generate'} onchange={()=>changeMode('generate')}/>Generate a new key</label>
    <label class="flex items-center gap-2"><input type="radio" name="signer-mode" value="backup" checked={mode==='backup'} onchange={()=>changeMode('backup')}/>Generate with backup</label>
    <label class="flex items-center gap-2"><input type="radio" name="signer-mode" value="import" checked={mode==='import'} onchange={()=>changeMode('import')}/>Import an existing key</label>
    <label class="flex items-center gap-2"><input type="radio" name="signer-mode" value="manual" checked={mode==='manual'} onchange={()=>changeMode('manual')}/>Advanced: existing Secret reference</label>
   </div></fieldset>{/if}
   {#if mode==='generate'}<p class="text-sm text-muted-foreground">The server generates the key and stores it in an immutable signing Secret. The private key is never returned to this browser.</p>{/if}
   {#if mode==='backup'&&!setup}
    <p class="text-sm text-muted-foreground">The signing key certifies runtime images. Download the private key once and store it in a trusted secret store. The public key remains available for your repository trust policy.</p>
    {#if backupDownloaded}
     <p role="status" class="text-sm">Download requested. Check that you have the file before saving the signer.</p>
    {:else}
     <Button type="button" variant="outline" disabled={busy} onclick={downloadBackup}>Download private key</Button>
    {/if}
   {/if}
   {#if mode==='import'&&(!setup||canResume)}<div class="space-y-4">
    <p class="text-sm text-muted-foreground">{setup?'Re-enter the same seed to resume this retained identity.':'Import the raw value of AYNI_ENV_CERTIFICATE_SIGNING_KEY. Use 64 lowercase hexadecimal characters, without an environment assignment or quotes.'}</p>
    <div class="space-y-2"><Label for="signer-seed">{setup?'Same private signing seed':'Private signing seed'}</Label><Input id="signer-seed" type="password" autocomplete="off" bind:value={seed} maxlength={65} disabled={busy} aria-describedby="signer-seed-help"/><p id="signer-seed-help" class="text-xs text-muted-foreground">Paste the seed or choose its raw file. It is cleared after submission and is never saved in browser storage.</p></div>
    <div class="space-y-2"><Label for="signer-seed-file">Signing seed file</Label><input id="signer-seed-file" type="file" bind:this={seedFile} onchange={readSeedFile} disabled={busy} class="block w-full rounded-lg border border-input bg-transparent px-2.5 py-1 text-sm file:mr-3 file:rounded file:border-0 file:bg-muted file:px-2 file:py-1 file:text-foreground disabled:opacity-50"/></div>
   </div>{/if}
   {#if mode==='manual'}<div class="space-y-4">
    <div class="space-y-2"><Label for="signer-public-key">Public key</Label><Input id="signer-public-key" bind:value={publicKey} required minlength={64} maxlength={64} pattern={'[0-9a-f]{64}'} disabled={busy||hasPending}/></div>
    <div class="space-y-2"><Label for="signer-secret-name">Immutable Secret name</Label><Input id="signer-secret-name" bind:value={secretName} required maxlength={63} pattern={'ayni-runtime-signer-[a-z0-9]([a-z0-9\\-]*[a-z0-9])?'} disabled={busy||hasPending}/></div>
    <div class="space-y-2"><Label for="signer-secret-uid">Secret UID</Label><Input id="signer-secret-uid" bind:value={secretUid} required maxlength={36} pattern={'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'} disabled={busy||hasPending}/></div>
    <p class="text-sm text-muted-foreground">Use a public key and immutable Secret reference already provisioned by the administrator. Secret access is checked during preparation.</p>
   </div>{/if}
   <p class="text-sm text-muted-foreground">Setting up a signer does not change repository trust or authorize a build. Review and commit the public policy, regenerate the lock, then inspect that commit.</p>
   <div class="flex flex-wrap gap-2">{#if !setup}<Button type="submit" disabled={busy||!!pending||(mode==='backup'&&!backupDownloaded)}>{mode==='backup'?'Save signer':mode==='manual'?'Save signer reference':mode==='import'?'Import signer':'Generate signer'}</Button>{:else if canResume&&setup.mode==='import'}<Button type="submit" disabled={busy}>Resume signer setup</Button>{/if}<Button type="button" variant="outline" disabled={busy||hasPending} onclick={closeEditor}>Cancel</Button></div>
  </form>
 </InformationCard>
 {/if}
 <p class="text-sm text-muted-foreground">Disabling a signer prevents new use; it does not delete its stored key.</p>
 <Button variant="outline" disabled={busy} onclick={()=>perform(()=>load())}>Refresh signers</Button>
 {#if busy}<p role="status" class="text-sm text-muted-foreground">Loading signer configuration…</p>{/if}
 {#if !signers.length&&!busy}<p class="text-sm text-muted-foreground">Add a named signing key to get started. You can configure multiple keys.</p>{/if}
 {#each signers as item(item.id)}<InformationCard title={item.keyId} status={item.enabled?'enabled':'disabled'}>
  <InformationTable rows={[{label:'Public key',value:item.publicKey,monospace:true},{label:'Secret name',value:item.secretName},{label:'Secret UID',value:item.secretUid,monospace:true},{label:'Configuration revision',value:item.revision}]}/>
  <details class="space-y-3"><summary class="cursor-pointer text-sm font-medium">Repository trust policy</summary><pre class="overflow-x-auto rounded-lg bg-muted p-3 text-xs">{trustSnippet(item)}</pre><Button variant="outline" onclick={()=>copyTrust(item)}>Copy public trust policy</Button><p class="text-xs text-muted-foreground">Merge this public entry into the existing environment.certificate.trusted_keys configuration in .ayni.toml, preserving other keys. If that configuration uses inline tables, edit the existing entry rather than adding a duplicate table. Commit the policy and regenerated lock, then inspect the new commit.</p></details>
  <div class="flex flex-wrap gap-2"><Button variant="outline" onclick={()=>copyPublicKey(item)}>Copy public key</Button><Button variant="outline" disabled={busy||hasPending} onclick={()=>select(item)}>Edit {item.keyId}</Button><Button variant="outline" disabled={busy||hasPending} onclick={()=>perform(()=>toggle(item))}>{item.enabled?'Disable':'Enable'} {item.keyId}</Button></div>
 </InformationCard>{/each}
 {#if next}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>load(true))}>More signers</Button>{/if}
</div>
