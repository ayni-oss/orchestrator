<script lang="ts">
 import { onMount } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Textarea } from '$lib/components/ui/textarea/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import InformationCard from '$lib/components/information-card.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { rpc, reasonText, type Connection, type Operation } from '$lib/api';
 let connections = $state.raw<Connection[]>([]);
 let history = $state.raw<Operation[]>([]);
 let selected = $state<Connection | null>(null);
 let name = $state(''); let appId = $state(''); let pem = $state('');
 let open=$state(false);
 let busy = $state(true); let error = $state(''); let notice = $state('');
 let pendingKey = $state(''); let next = $state(''); let historyNext = $state('');
 let selectionGeneration = 0;
 const storageKey = 'ayni.connection.pending';
 function persistKey(key: string) { pendingKey = key; if (key) sessionStorage.setItem(storageKey, key); else sessionStorage.removeItem(storageKey); }
 async function load(append = false) {
   const result = await rpc('ListConnections', { afterId: append ? next : '', limit: 50 });
   connections = append ? [...connections, ...(result.connections || [])] : result.connections || []; next = result.nextAfterId || '';
 }
 async function inspect(c: Connection, append = false) {
   const generation = ++selectionGeneration;
   if (!append) { selected = c; name = c.displayName; appId = c.appId; pem = ''; history = []; }
   const result = await rpc('ListConnectionOperations', { connectionId: c.id, afterId: append ? historyNext : '', limit: 50 });
   if (generation !== selectionGeneration) return;
   history = append ? [...history, ...(result.operations || [])] : result.operations || []; historyNext = result.nextAfterId || '';
 }
 async function refresh() {
   await load();
   if (selected) { const current = connections.find(c => c.id === selected?.id); if (current) await inspect(current); }
 }
 async function perform(action: () => Promise<void>) {
   busy = true; error = '';
   try { await action(); } catch (e) { error = e instanceof Error ? e.message : 'Service unavailable. Retry the read.'; }
   finally { busy = false; }
 }
 async function accept(operation: Operation) {
   const message = operation.kind === 'enable' ? (operation.connection.enabled ? 'Connection enabled.' : 'Connection disabled.') : ({ accepted: 'Candidate validation is pending.', completed: 'GitHub App verified.', failed: 'Candidate validation failed.', superseded: 'This candidate was superseded. Refresh the connection.' }[operation.state] || 'Operation recorded.');
   notice = `${message} ${reasonText[operation.safeReason || ''] || ''}`;
   await load(); await inspect(operation.connection);
 }
 async function save(event: SubmitEvent) {
   event.preventDefault();
   if (pendingKey) return;
   const key = crypto.randomUUID();
   const request = { idempotencyKey: key, connectionId: selected?.id || '', expectedRevision: selected?.revision || '0', displayName: name, appId, privateKeyPem: pem };
   pem = ''; persistKey(key);
   await perform(async () => {
     try {
       const result = await rpc('SaveGithubConnection', request);
       request.privateKeyPem = ''; persistKey(''); open=false; await accept(result.operation);
       notice = 'Credentials saved. Validate the candidate to activate this connection.';
     } finally { request.privateKeyPem = ''; }
   });
 }
 async function recover() {
   await perform(async () => {
     const result = await rpc('GetConnectionOperation', { idempotencyKey: pendingKey });
     persistKey(''); await accept(result.operation);
   });
 }
 function createNew() { open=true; ++selectionGeneration; selected = null; name = ''; appId = ''; pem = ''; history = []; notice = ''; error = ''; }
 onMount(() => { pendingKey = sessionStorage.getItem(storageKey) || ''; void perform(refresh); });
</script>
<div class="space-y-4">
 <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">GitHub App</h2><Button disabled={busy||!!pendingKey} onclick={createNew}>Add GitHub App</Button></div>
 {#if error}<Feedback kind="error" title="Connection needs attention" message={error}/><Button variant="outline" disabled={busy} onclick={()=>perform(refresh)}>Refresh</Button>{/if}
 {#if notice}<Feedback title="Connection update" message={notice}/>{/if}
 {#if pendingKey}<InformationCard title="Recover a submitted command"><p>The response was interrupted. Check its retained receipt before submitting credentials again.</p><code>{pendingKey}</code><div class="flex gap-3"><Button disabled={busy} onclick={recover}>Recover operation</Button><Button variant="outline" disabled={busy} onclick={()=>persistKey('')}>Abandon recovery</Button></div></InformationCard>{/if}
 {#if !connections.length}<p class="text-sm text-muted-foreground">Add a GitHub App to connect your repositories.</p>{/if}
 {#each connections as c (c.id)}
  <InformationCard title={c.displayName} status={c.status}>
   <InformationTable rows={[{label:'App ID',value:c.appId},{label:'Credential',value:c.activeCredentialRevision?'Configured':'No active credential'},{label:'Revision',value:c.revision}]}/>
   {#if c.safeReason}<Feedback kind="warning" title="Latest operation needs attention" message={reasonText[c.safeReason]||c.safeReason}/>{/if}
   <div class="flex flex-wrap gap-3"><Button variant="outline" disabled={busy} onclick={()=>perform(async()=>{await inspect(c);open=true;})}>Edit {c.displayName}</Button><Button variant="outline" disabled={busy} onclick={()=>perform(()=>inspect(c))}>Inspect {c.displayName}</Button></div>
  </InformationCard>
 {/each}
 {#if next}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>load(true))}>Load more connections</Button>{/if}
 <Button variant="outline" disabled={busy} onclick={()=>perform(refresh)}>Refresh connections</Button>
</div>
<Dialog.Root bind:open onOpenChange={value=>{if(!value)pem='';}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>{selected?'Connection settings':'Add a GitHub App'}</Dialog.Title><Dialog.Description>Enter credentials from an existing GitHub App.</Dialog.Description></Dialog.Header>
 <form onsubmit={save} class="space-y-5">
  <div class="space-y-2"><Label for="connection-name">Connection name</Label><Input id="connection-name" bind:value={name} maxlength={120} required disabled={busy||!!pendingKey}/></div>
  <div class="space-y-2"><Label for="app-id">GitHub App ID</Label><Input id="app-id" bind:value={appId} pattern="[1-9][0-9]*" maxlength={20} required disabled={busy||!!pendingKey||!!selected}/></div>
  <div class="space-y-2"><Label for="private-key">{selected?'Replacement private key (PEM)':'Private key (PEM)'}</Label><Textarea id="private-key" bind:value={pem} rows={6} maxlength={32768} required autocomplete="off" spellcheck={false} disabled={busy||!!pendingKey} class="font-mono text-xs"/></div>
  <p class="text-xs text-muted-foreground">Stored in PostgreSQL. Hosting controls storage encryption. Keys are never returned.</p>
  <Dialog.Footer><Button type="button" variant="outline" onclick={()=>open=false}>Cancel</Button><Button type="submit" disabled={busy||!!pendingKey}>{selected?'Save candidate key':'Save connection'}</Button></Dialog.Footer>
 </form>
</Dialog.Content></Dialog.Root>
{#if selected}<section class="mt-8 space-y-4"><h3 class="text-lg font-semibold">Operation history</h3><p class="text-sm text-muted-foreground">App authentication and repository access are separate. Validation does not establish runtime readiness.</p>
 <Button variant="outline" disabled={busy||!!pendingKey} onclick={()=>perform(async()=>{const key=crypto.randomUUID();persistKey(key);const result=await rpc('SetConnectionEnabled',{idempotencyKey:key,connectionId:selected!.id,expectedRevision:selected!.revision,enabled:!selected!.enabled});persistKey('');await accept(result.operation);})}>{selected.enabled?'Disable connection':'Enable connection'}</Button>
 {#each history as op (op.id)}<InformationCard title={op.kind==='save'?'Credential candidate':'Connection state'} status={op.state}><InformationTable rows={[{label:'Operation',value:op.id,monospace:true},{label:'Revision',value:op.connectionRevision}]}/>{#if op.safeReason}<p class="text-sm">{reasonText[op.safeReason]||op.safeReason}</p>{/if}{#if op.kind==='save'&&op.state==='accepted'}<Button disabled={busy} onclick={()=>perform(async()=>{const result=await rpc('ValidateGithubConnection',{operationId:op.id});await accept(result.operation);})}>Validate candidate</Button>{/if}</InformationCard>{/each}
 {#if historyNext}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>inspect(selected!,true))}>Load more history</Button>{/if}
</section>{/if}
