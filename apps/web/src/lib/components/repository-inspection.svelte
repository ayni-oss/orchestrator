<script lang="ts">
 import { onDestroy, onMount } from 'svelte';
 import { RefreshCw } from '@lucide/svelte';
 import { Tabs } from 'bits-ui';
 import { toast } from 'svelte-sonner';
 import ActivityEntry from '$lib/components/activity-entry.svelte';
 import HistoryPages from '$lib/components/history-pages.svelte';
 import AutosaveField from '$lib/components/autosave-field.svelte';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import Feedback from '$lib/components/feedback.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import RuntimePreparation from '$lib/components/runtime-preparation.svelte';
 import type { StatusTone } from '$lib/presentation.js';
 import { readRuntimeProgress } from '$lib/runtime-preparation';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import { inspectionReasons, type Repository, type RepositoryInspection, type InspectRequest } from '$lib/repositories';
 import { historyBatchSize, readRecentHistory, type HistoryPage } from '$lib/repository-history';
 let { repo, connection = $bindable() }: { repo: Repository; connection?: Connection } = $props();
 let branch = $state('');
 let hasBranchPreference = false;
 let branchSavedRevision = $state(0);
 let branches = $state.raw<string[]>([]);
 let nextBranchPage = $state(0);
 let loadingBranches = $state(false);
 let branchError = $state('');
 let checked = $state<{branch: string; sha: string} | null>(null);
 let busy = $state(false);
 let error = $state('');
 let inspectionLoaded = $state(false);
 let checkingDefault = $state(false);
 type PendingInspection = InspectRequest & { branch?: string };
 let pending = $state<PendingInspection | null>(null);
 let mounted = false;
 let recoveryToast: string | number | undefined;
 let inspections = $state.raw<RepositoryInspection[]>([]);
 let next = $state('');
 let inspectionHistoryOpened = $state(false);
 let inspectionHistoryRevision = $state(0);
 const storageKey = $derived(`ayni.repository.inspection.pending.${repo.id}`);
 const branchStorageKey = $derived(`ayni.repository.runtime.branch.${repo.id}`);
 const canInspect = $derived(!!connection?.enabled && !!connection?.activeCredentialRevision);
 const currentInspection = $derived(inspections.find(value => value.commitSha === checked?.sha &&
  value.connectionRevision === connection?.revision && value.state === 'completed') ?? inspections[0] ?? null);
 const inspectionStates: Record<string, {label: string; tone: StatusTone}> = {
  completed:{label:'Complete',tone:'success'},
  accepted:{label:'Awaiting retry',tone:'warning'},
  superseded:{label:'Source connection changed',tone:'neutral'},
  failed:{label:'Failed',tone:'danger'},
 };
 function dismissRecoveryToast() {
  if (recoveryToast !== undefined) toast.dismiss(recoveryToast);
  recoveryToast = undefined;
 }
 function persist(value: PendingInspection | null) {
  pending = value;
  if (value) sessionStorage.setItem(storageKey, JSON.stringify(value));
  else {
   sessionStorage.removeItem(storageKey);
   dismissRecoveryToast();
  }
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
  } finally {
   busy = false;
   if (mounted && pending) {
    recoveryToast = toast.warning('Inspection result not confirmed', {
     id: recoveryToast, duration: Infinity,
     description: `The next check will recover the request for commit ${pending.commitSha.slice(0, 12)}. ${error}`.trim(),
     cancel: {label:'Discard request', onClick:() => { if (!busy) persist(null); }},
    });
   } else if (mounted && error) {
    toast.error('Could not complete the inspection', {description:error});
   }
  }
 }
 async function readInspections(afterId: string): Promise<HistoryPage<RepositoryInspection>> {
  const response = await rpc('ListRepositoryInspections', {repositoryId:repo.id, afterId, limit:historyBatchSize});
  return {items:response.inspections || [], next:response.nextAfterId || ''};
 }
 async function load() {
  const result = await readRecentHistory(readInspections);
  inspections = result.items;
  next = result.next;
  inspectionLoaded = true;
 }
 async function accept(value: RepositoryInspection) {
  inspections = [value, ...inspections.filter(i => i.id !== value.id)].sort((a,b) => Number(b.acceptedAtUnixMs || b.updatedAtUnixMs) - Number(a.acceptedAtUnixMs || a.updatedAtUnixMs) || b.id.localeCompare(a.id));
  inspectionHistoryRevision += 1;
  if (value.state !== 'accepted') persist(null);
 }
 async function acceptPending(value: RepositoryInspection, request: PendingInspection) {
  if (value.repositoryId !== request.repositoryId || value.commitSha !== request.commitSha) {
   throw new Error('The retained inspection did not match the requested commit.');
  }
  await accept(value);
  if (value.state === 'completed' && request.branch === branch.trim() &&
   request.expectedConnectionRevision === connection?.revision) {
   checked = {branch: request.branch, sha: request.commitSha};
  }
 }
 async function resume(request: PendingInspection) {
  const { branch: _branch, ...input } = request;
  const result = await rpc('InspectRepository', input);
  await acceptPending(result.inspection, request);
 }
 async function recoverOrRetry(request: PendingInspection) {
  try {
   const { inspection } = await rpc('GetRepositoryInspection', {idempotencyKey:request.idempotencyKey});
   if (inspection.state !== 'accepted') {
    await acceptPending(inspection, request);
    return;
   }
  } catch (cause) {
   if (!(cause instanceof RpcError) || cause.status !== 404) throw cause;
  }
  await resume(request);
 }
 async function loadBranches(page = 1) {
  if (!connection || !canInspect || loadingBranches) return;
  const revision = connection.revision;
  loadingBranches = true;
  branchError = '';
  try {
   const result = await rpc('ListRepositoryBranches', { repositoryId: repo.id, expectedConnectionRevision: revision, page });
   if (connection?.revision !== revision || !canInspect) return;
   if (result.safeReason) throw new Error(inspectionReasons[result.safeReason] || 'Check the source connection and repository access.');
   branches = [...new Set([...(page === 1 ? [] : branches), ...(result.branches || [])])];
   nextBranchPage = result.nextPage || 0;
  } catch (cause) {
   if (connection?.revision !== revision || !canInspect) return;
   branchError = cause instanceof RpcError && cause.status === 409
    ? 'The source connection changed. Reload this page to list branches.'
    : cause instanceof Error && !(cause instanceof RpcError) ? cause.message : 'Check the source connection and try again.';
  } finally {
   loadingBranches = false;
   if (canInspect && connection?.revision !== revision) void loadBranches();
  }
 }
 function refreshSource(value: Connection | undefined) {
  const changed = value?.revision !== connection?.revision || value?.enabled !== connection?.enabled ||
   value?.activeCredentialRevision !== connection?.activeCredentialRevision;
  connection = value;
  if (changed) {
   checked = null;
   branches = [];
   nextBranchPage = 0;
   branchError = '';
   if (canInspect) void loadBranches();
  }
 }
 async function resolveBranch(selectedBranch: string, revision: string): Promise<string> {
  const resolved = await rpc('ResolveRepositoryDefaultCommit', {repositoryId:repo.id, expectedConnectionRevision:revision, branch:selectedBranch});
  if (resolved.safeReason) throw new Error(inspectionReasons[resolved.safeReason] || 'The selected branch could not be resolved. Retry when the source connection is available.');
  if (!/^[a-f0-9]{40}$/.test(resolved.commitSha)) throw new Error('The source connection did not return an exact commit for the selected branch.');
  return resolved.commitSha;
 }
 async function restoreCheckedCommit() {
  if (!readRuntimeProgress(repo.id)?.inspection || !canInspect || !connection ||
      !inspections.some(value => value.state === 'completed' && value.connectionRevision === connection?.revision)) return;
  const selectedBranch = branch.trim(), revision = connection.revision;
  checkingDefault = true;
  try {
   const sha = await resolveBranch(selectedBranch, revision);
   if (mounted && selectedBranch === branch.trim() && revision === connection?.revision &&
       inspections.some(value => value.commitSha === sha && value.connectionRevision === revision && value.state === 'completed')) {
    checked = {branch:selectedBranch, sha};
   }
  } catch {
   // Keep the last report visible and let the user check again when the branch is reachable.
  } finally {
   checkingDefault = false;
  }
 }
 async function recheckDefaultBranch() {
  if (!pending && !connection) return;
  checkingDefault = true;
  try {
   if (pending) return await recoverOrRetry(pending);
   if (!connection) return;
   checked = null;
   const selectedBranch = branch.trim();
   const revision = connection.revision;
   const sha = await resolveBranch(selectedBranch, revision);
   if (selectedBranch !== branch.trim() || revision !== connection?.revision) return;
   const request = {idempotencyKey:crypto.randomUUID(), repositoryId:repo.id, expectedConnectionRevision:revision, commitSha:sha, branch:selectedBranch};
   persist(request);
   await resume(request);
  } finally { checkingDefault = false; }
 }
 function restoreBranch(value?: string) {
  if (!hasBranchPreference) branch = value === repo.defaultBranch ? '' : value || '';
 }
 function saveBranch(event: Event & { currentTarget: HTMLSelectElement }) {
  const select = event.currentTarget;
  const value = select.value;
  try {
   localStorage.setItem(branchStorageKey, value);
   hasBranchPreference = true;
   branch = value;
   checked = null;
   branchSavedRevision += 1;
  } catch {
   select.value = branch;
   toast.error('Could not save the runtime branch', { description: 'Check that this browser allows site storage, then try again.' });
  }
 }
 onMount(() => {
  mounted = true;
  try {
   const savedBranch = localStorage.getItem(branchStorageKey) ?? sessionStorage.getItem(branchStorageKey);
   if (savedBranch !== null && savedBranch.length <= 255) {
    hasBranchPreference = true;
    branch = savedBranch;
   }
  } catch {
   // The branch remains at its default when browser storage is unavailable.
  }
  void loadBranches();
  const saved = sessionStorage.getItem(storageKey);
  if (!saved) { void perform(async () => { await load(); await restoreCheckedCommit(); }); return; }
  try {
   const value = JSON.parse(saved);
   if (value.repositoryId !== repo.id || !/^[a-f0-9]{40}$/.test(value.commitSha) || typeof value.idempotencyKey !== 'string' || typeof value.expectedConnectionRevision !== 'string' || (value.branch !== undefined && typeof value.branch !== 'string')) throw new Error();
   pending = value;
   void perform(() => load());
  } catch { error = 'The saved inspection request could not be read. Reload repository history before starting another.'; }
 });
 onDestroy(() => {
  mounted = false;
  dismissRecoveryToast();
 });
</script>
{#snippet branchPicker()}
 <div class="space-y-2">
  <Label for={`runtime-branch-${repo.id}`}>Runtime branch</Label>
  <AutosaveField savedRevision={branchSavedRevision}>
   <NativeSelect.Root data-slot="input-group-control" id={`runtime-branch-${repo.id}`} class="w-full" value={branch}
    onchange={saveBranch} disabled={busy || !!pending}>
    <NativeSelect.Option value="">Default branch</NativeSelect.Option>
    {#if branch && !branches.includes(branch)}
     <NativeSelect.Option value={branch}>{branch} (saved selection)</NativeSelect.Option>
    {/if}
    {#each branches as name (name)}<NativeSelect.Option value={name}>{name}</NativeSelect.Option>{/each}
   </NativeSelect.Root>
  </AutosaveField>
  {#if loadingBranches}<p role="status" class="sr-only">Loading branches…</p>
  {:else if branchError}
   <Feedback kind="error" title="Could not list branches" message={branchError}/>
   <Button variant="outline" onclick={() => loadBranches(nextBranchPage || 1)}>Retry branch list</Button>
  {:else if nextBranchPage}
   <Button variant="outline" onclick={() => loadBranches(nextBranchPage)}>More branches</Button>
  {/if}
 </div>
{/snippet}

{#snippet inspectionEntry(item: RepositoryInspection)}
   {@const state = inspectionStates[item.state] ?? inspectionStates.failed}
   {@const result = item.result}
   {#snippet inspectionActions()}
    <Button variant="outline" disabled={busy || !!pending} onclick={() => perform(async () => {
     const request = {idempotencyKey:item.idempotencyKey, repositoryId:item.repositoryId, expectedConnectionRevision:item.connectionRevision, commitSha:item.commitSha};
     persist(request);
     await resume(request);
    })}>Resume inspection</Button>
   {/snippet}
   <ActivityEntry label={`Inspection ${item.commitSha}`} updatedAtUnixMs={item.updatedAtUnixMs}
    identifiers={[
     {label:'Inspection ID', value:item.id},
     {label:result?.lockFingerprint ? 'Lock fingerprint' : 'Lock digest', value:result?.lockFingerprint || result?.lockDigest},
    ]}
    title="Inspection details" description={state.label} statuses={[{label:'Status', status:state.label, tone:state.tone}]}
    actions={item.state === 'accepted' ? inspectionActions : undefined}>
    {#snippet information()}
     <span class="text-muted-foreground">Commit</span> <code title={item.commitSha}>{item.commitSha.slice(0,12)}</code>
    {/snippet}
    {#snippet details(fullTime)}
     {#if item.safeReason}<p>{inspectionReasons[item.safeReason] || 'Source inspection needs attention. Check the source connection and retry.'}</p>{/if}
     <InformationTable aligned rows={[
      {label:'Commit',value:item.commitSha,monospace:true},
      {label:'Inspection ID',value:item.id,monospace:true},
      {label:'Source connection revision',value:item.connectionRevision},
      {label:'Updated',value:fullTime},
      ...(result ? [
       {label:'Policy digest',value:result.policyDigest || 'Unavailable',monospace:true},
       {label:'Lock digest',value:result.lockDigest || 'Unavailable',monospace:true},
       {label:'Lock fingerprint',value:result.lockFingerprint || 'Not validated',monospace:true},
       {label:'ayni version',value:result.ayniVersion || 'Unavailable'},
       {label:'Lock schema',value:result.lockSchema || 'Unavailable'},
       {label:'Locked tools',value:result.tools?.join(', ') || 'None inspected'},
       {label:'Trusted key IDs',value:result.trustKeyIds?.join(', ') || 'None inspected'},
      ] : []),
     ]}/>
     {#if result?.findings?.length}
      <h3 class="border-t pt-3 font-medium">Inspection findings</h3>
      {#each result.findings as finding (finding.code + finding.path)}
       <div class="space-y-1">
        <p class="font-medium break-all">{finding.path || 'Preparation requirements'}</p>
        <p>{finding.action}</p>
        <code class="break-all text-xs text-muted-foreground">{finding.code}</code>
       </div>
      {/each}
     {/if}
    {/snippet}
   </ActivityEntry>
{/snippet}

{#snippet inspectionHistory()}
 <div class="relative">
 <details class="rounded-xl border p-4" data-inspection-history
  ontoggle={(event) => { if (event.currentTarget.open) inspectionHistoryOpened = true; }}>
  <summary class="cursor-pointer pr-10 text-sm font-medium">Inspection history ({inspections.length}{next ? '+' : ''})</summary>
  {#if inspectionHistoryOpened}
   <div class="space-y-2 pt-4">
    <p class="text-sm text-muted-foreground">Saved checks are historical evidence. An older awaiting-retry check does not block the latest completed check.</p>
     <HistoryPages revision={inspectionHistoryRevision} label="Inspection history pages" empty="No inspections yet." readPage={readInspections}>
      {#snippet children(items)}
       {#each items as item (item.id)}{@render inspectionEntry(item)}{/each}
      {/snippet}
     </HistoryPages>
   </div>
  {/if}
 </details>
 <Button class="absolute top-3 right-4" variant="ghost" size="icon-sm" aria-label="Refresh inspection history" title="Refresh inspection history" aria-busy={busy} disabled={busy}
  onclick={() => perform(async () => { await load(); inspectionHistoryRevision += 1; })}>
  <RefreshCw class={{ 'motion-safe:animate-spin': busy }} aria-hidden="true" />
 </Button>
 </div>
{/snippet}

<div data-repository-inspection={repo.id}>
 <RuntimePreparation {repo} {connection} {branchPicker} onSourceRefresh={refreshSource} onRestoreBranch={restoreBranch} branch={branch.trim()} checkedCommit={checked?.branch === branch.trim() ? checked?.sha : undefined} latestInspection={currentInspection} inspectionLoading={!inspectionLoaded && !error} inspectionError={error} checkingDefault={checkingDefault} recheckDisabled={busy || (!pending && !canInspect)} inspectionPending={!!pending} refreshInspections={() => load()} recheckDefaultBranch={() => perform(recheckDefaultBranch)}>
  {#snippet children(runtime, currentPreparation, preparationHistory)}
   <Tabs.Content value="runtime" class="space-y-6">
    {@render runtime()}
   </Tabs.Content>
   <Tabs.Content value="status" class="space-y-6">
     {@render currentPreparation()}
     {@render preparationHistory()}
     {@render inspectionHistory()}
   </Tabs.Content>
  {/snippet}
 </RuntimePreparation>
</div>
