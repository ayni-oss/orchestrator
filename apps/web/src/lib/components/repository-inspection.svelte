<script lang="ts">
 import { onMount } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import Feedback from '$lib/components/feedback.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import { inspectionReasons, type Repository, type RepositoryInspection, type InspectRequest } from '$lib/repositories';
 let { repo, connection }: { repo: Repository; connection?: Connection } = $props();
 let expanded = $state(false);
 let commitSha = $state('');
 let busy = $state(false);
 let error = $state('');
 let pending = $state<InspectRequest | null>(null);
 let inspections = $state.raw<RepositoryInspection[]>([]);
 let next = $state('');
 const storageKey = $derived(`ayni.repository.inspection.pending.${repo.id}`);
 const canInspect = $derived(!!connection?.enabled && !!connection?.activeCredentialRevision);
 const validCommit = $derived(/^[a-f0-9]{40}$/.test(commitSha));
 function persist(value: InspectRequest | null) {
  pending = value;
  if (value) sessionStorage.setItem(storageKey, JSON.stringify(value));
  else sessionStorage.removeItem(storageKey);
 }
 async function perform(action: () => Promise<void>) {
  busy = true; error = '';
  try { await action(); }
  catch (e) {
   error = e instanceof RpcError && e.status === 409
    ? 'The source connection or retained request has changed. Recover the pending inspection, then refresh repositories before starting another.'
    : e instanceof RpcError && e.status === 404
    ? 'The inspection has not been found. Retry the same request if its response was interrupted.'
    : e instanceof Error ? e.message : 'Inspection could not be loaded. Retry to recover the retained result.';
  } finally { busy = false; }
 }
 async function load(append = false) {
  const response = await rpc('ListRepositoryInspections', { repositoryId: repo.id, afterId: append ? next : '', limit: 20 });
  inspections = append ? [...inspections, ...(response.inspections || [])] : response.inspections || [];
  next = response.nextAfterId || '';
 }
 async function accept(value: RepositoryInspection) {
  inspections = [value, ...inspections.filter(i => i.id !== value.id)];
  if (value.state !== 'accepted') persist(null);
 }
 async function resume() {
  if (!pending) return;
  const result = await rpc('InspectRepository', pending);
  await accept(result.inspection);
 }
 async function inspect() {
  if (!connection || pending || !validCommit) return;
  persist({ idempotencyKey: crypto.randomUUID(), repositoryId: repo.id, expectedConnectionRevision: connection.revision, commitSha });
  await resume();
 }
 async function recover() {
  if (!pending) return;
  const result = await rpc('GetRepositoryInspection', { idempotencyKey: pending.idempotencyKey });
  await accept(result.inspection);
 }
 async function open() { expanded = true; await load(); }
 onMount(() => {
  const saved = sessionStorage.getItem(storageKey);
  if (!saved) return;
  try {
   const value = JSON.parse(saved);
   if (value.repositoryId !== repo.id || !/^[a-f0-9]{40}$/.test(value.commitSha) || typeof value.idempotencyKey !== 'string' || typeof value.expectedConnectionRevision !== 'string') throw new Error();
   pending = value; expanded = true;
   void perform(() => load());
  } catch { error = 'The saved inspection request could not be read. Reload repository history before starting another.'; expanded = true; }
 });
</script>
<div class="space-y-4 border-t pt-4" data-repository-inspection={repo.id}>
 {#if !expanded}
  <Button variant="outline" onclick={() => perform(open)} disabled={busy}>Inspect Ayni setup</Button>
 {:else}
  <h3 class="font-semibold">Ayni inspection</h3>
  <p class="text-sm text-muted-foreground">Inspect committed policy, lock and public trust requirements at an exact commit. Inspection does not enable runtime preparation or start tasks.</p>
  {#if error}<Feedback kind="error" title="Inspection needs attention" message={error} />{/if}
  {#if busy}<p role="status" class="text-sm text-muted-foreground">Loading inspection…</p>{/if}
  {#if pending}
   <div class="space-y-3 rounded-md border p-4">
    <p class="font-medium">Recover Ayni inspection</p>
    <p class="break-all text-sm">Commit: {pending.commitSha}</p>
    <p class="text-sm text-muted-foreground">The original request is retained. Recover its result or retry that exact inspection.</p>
    <div class="flex flex-wrap gap-2">
     <Button disabled={busy} onclick={() => perform(recover)}>Recover inspection</Button>
     <Button variant="outline" disabled={busy} onclick={() => perform(resume)}>Retry same inspection</Button>
     <Button variant="outline" disabled={busy} onclick={() => persist(null)}>Dismiss inspection recovery</Button>
    </div>
   </div>
  {/if}
  <div class="space-y-2">
   <Label for={`inspection-commit-${repo.id}`}>Commit SHA</Label>
   <Input id={`inspection-commit-${repo.id}`} bind:value={commitSha} placeholder="Full 40-character lowercase commit SHA" disabled={busy || !!pending} maxlength={40} />
   <p class="text-xs text-muted-foreground">Copy the full commit SHA from GitHub. Branches and tags are not accepted.</p>
  </div>
  {#if !canInspect}<p class="text-sm">Enable and validate the repository's source connection before starting an inspection.</p>{/if}
  <div class="flex flex-wrap gap-2">
   <Button disabled={busy || !!pending || !canInspect || !validCommit} onclick={() => perform(inspect)}>Inspect commit</Button>
   <Button variant="outline" disabled={busy} onclick={() => perform(() => load())}>Refresh inspections</Button>
  </div>
  {#if !inspections.length && !busy}<p class="text-sm text-muted-foreground">No retained inspections for this repository.</p>{/if}
  {#each inspections as item (item.id)}
   <section class="space-y-3 rounded-md border p-4" aria-label={`Inspection ${item.commitSha}`}>
    <p class="font-medium">{item.state === 'completed' ? 'Static inspection complete' : item.state === 'accepted' ? 'Inspection awaiting retry' : item.state === 'superseded' ? 'Source connection changed' : 'Inspection failed'}</p>
    <InformationTable rows={[
     { label: 'Commit', value: item.commitSha, monospace: true },
     { label: 'Operation', value: item.id, monospace: true },
     { label: 'Source connection revision', value: item.connectionRevision },
     { label: 'Updated', value: new Date(Number(item.updatedAtUnixMs)).toLocaleString() },
    ]} />
    {#if item.safeReason}<p class="text-sm">{inspectionReasons[item.safeReason] || 'Source inspection needs attention. Check the source connection and retry.'}</p>{/if}
    {#if item.result}
     <InformationTable rows={[
      { label: 'Policy digest', value: item.result.policyDigest || 'Unavailable', monospace: true },
      { label: 'Lock digest', value: item.result.lockDigest || 'Unavailable', monospace: true },
      { label: 'Lock fingerprint', value: item.result.lockFingerprint || 'Not validated', monospace: true },
      { label: 'Ayni version', value: item.result.ayniVersion || 'Unavailable' },
      { label: 'Lock schema', value: item.result.lockSchema || 'Unavailable' },
      { label: 'Locked tools', value: item.result.tools?.join(', ') || 'None inspected' },
      { label: 'Trusted key IDs', value: item.result.trustKeyIds?.join(', ') || 'None inspected' },
     ]} />
     {#each item.result.findings || [] as finding (finding.code + finding.path)}
      <div class="space-y-1 border-t pt-3"><p class="text-sm font-medium">{finding.path || 'Preparation requirements'}</p><p class="text-sm">{finding.action}</p><code class="text-xs text-muted-foreground">{finding.code}</code></div>
     {/each}
    {/if}
    {#if item.state === 'accepted'}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(async () => {persist({ idempotencyKey: item.idempotencyKey, repositoryId: item.repositoryId, expectedConnectionRevision: item.connectionRevision, commitSha: item.commitSha }); await resume();})}>Resume inspection</Button>{/if}
   </section>
  {/each}
  {#if next}<Button variant="outline" disabled={busy} onclick={() => perform(() => load(true))}>More inspections</Button>{/if}
 {/if}
</div>
