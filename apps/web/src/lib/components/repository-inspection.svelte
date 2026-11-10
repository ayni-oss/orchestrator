<script lang="ts">
 import { onMount } from 'svelte';
 import { Tabs } from 'bits-ui';
 import { toast } from 'svelte-sonner';
 import AutosaveField from '$lib/components/autosave-field.svelte';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import Feedback from '$lib/components/feedback.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import RuntimePreparation from '$lib/components/runtime-preparation.svelte';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import { inspectionReasons, type Repository, type RepositoryInspection, type InspectRequest } from '$lib/repositories';
 let { repo, connection }: { repo: Repository; connection?: Connection } = $props();
 let expanded = $state(false);
 let commitSha = $state('');
 let branch = $state('');
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
 let pending = $state<InspectRequest | null>(null);
 let inspections = $state.raw<RepositoryInspection[]>([]);
 let next = $state('');
 const storageKey = $derived(`ayni.repository.inspection.pending.${repo.id}`);
 const branchStorageKey = $derived(`ayni.repository.runtime.branch.${repo.id}`);
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
  next = response.nextAfterId || ''; inspectionLoaded = true;
 }
 async function accept(value: RepositoryInspection) {
  inspections = [value, ...inspections.filter(i => i.id !== value.id)].sort((a,b) => Number(b.acceptedAtUnixMs || b.updatedAtUnixMs) - Number(a.acceptedAtUnixMs || a.updatedAtUnixMs) || b.id.localeCompare(a.id));
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
 async function loadBranches(page = 1) {
  if (!connection || !canInspect || loadingBranches) return;
  loadingBranches = true;
  branchError = '';
  try {
   const result = await rpc('ListRepositoryBranches', { repositoryId: repo.id, expectedConnectionRevision: connection.revision, page });
   if (result.safeReason) throw new Error(inspectionReasons[result.safeReason] || 'Check the source connection and repository access.');
   branches = [...new Set([...(page === 1 ? [] : branches), ...(result.branches || [])])];
   nextBranchPage = result.nextPage || 0;
  } catch (cause) {
   branchError = cause instanceof RpcError && cause.status === 409
    ? 'The source connection changed. Reload this page to list branches.'
    : cause instanceof Error && !(cause instanceof RpcError) ? cause.message : 'Check the source connection and try again.';
  } finally { loadingBranches = false; }
 }
 async function recheckDefaultBranch() {
  if (!connection || pending) return;
  checkingDefault = true;
  checked = null;
  const selectedBranch = branch.trim();
  try {
   const resolved = await rpc('ResolveRepositoryDefaultCommit', {repositoryId:repo.id, expectedConnectionRevision:connection.revision,branch:selectedBranch});
   if (resolved.safeReason) throw new Error(inspectionReasons[resolved.safeReason] || 'The selected branch could not be resolved. Retry when the source connection is available.');
   if (!/^[a-f0-9]{40}$/.test(resolved.commitSha)) throw new Error('The source connection did not return an exact commit for the selected branch.');
   commitSha = resolved.commitSha;
   persist({idempotencyKey:crypto.randomUUID(), repositoryId:repo.id, expectedConnectionRevision:connection.revision, commitSha:resolved.commitSha});
   await resume();
   checked = {branch: selectedBranch, sha: resolved.commitSha};
  } finally { checkingDefault = false; }
 }
 async function recover() {
  if (!pending) return;
  const result = await rpc('GetRepositoryInspection', { idempotencyKey: pending.idempotencyKey });
  await accept(result.inspection);
 }
 function saveBranch(event: Event & { currentTarget: HTMLSelectElement }) {
  const select = event.currentTarget;
  const value = select.value;
  try {
   localStorage.setItem(branchStorageKey, value);
   branch = value;
   checked = null;
   branchSavedRevision += 1;
  } catch {
   select.value = branch;
   toast.error('Could not save the runtime branch', { description: 'Check that this browser allows site storage, then try again.' });
  }
 }
 onMount(() => {
  try {
   const savedBranch = localStorage.getItem(branchStorageKey) ?? sessionStorage.getItem(branchStorageKey);
   if (savedBranch && savedBranch.length <= 255) branch = savedBranch;
  } catch {
   // The branch remains at its default when browser storage is unavailable.
  }
  void loadBranches();
  const saved = sessionStorage.getItem(storageKey);
  if (!saved) { void perform(() => load()); return; }
  try {
   const value = JSON.parse(saved);
   if (value.repositoryId !== repo.id || !/^[a-f0-9]{40}$/.test(value.commitSha) || typeof value.idempotencyKey !== 'string' || typeof value.expectedConnectionRevision !== 'string') throw new Error();
   pending = value;
   void perform(() => load());
  } catch { error = 'The saved inspection request could not be read. Reload repository history before starting another.'; }
 });
</script>
{#snippet recovery()}
 {#if pending}
  <section class="space-y-3 rounded-xl border bg-card p-5" aria-label="Current inspection recovery">
   <h3 class="font-semibold">Finish the interrupted check</h3>
   <p class="text-sm">The result for commit <code>{pending.commitSha.slice(0,12)}</code> is not confirmed. Recover it before checking a newer commit.</p>
   <div class="flex flex-wrap gap-2"><Button disabled={busy} onclick={() => perform(recover)}>Recover inspection</Button><Button variant="outline" disabled={busy} onclick={() => perform(resume)}>Retry same inspection</Button><Button variant="outline" disabled={busy} onclick={() => persist(null)}>Dismiss inspection recovery</Button></div>
  </section>
 {/if}
{/snippet}

{#snippet branchPicker()}
 <div class="space-y-2">
  <Label for={`runtime-branch-${repo.id}`}>Runtime branch</Label>
  <AutosaveField savedRevision={branchSavedRevision}>
   <NativeSelect.Root data-slot="input-group-control" id={`runtime-branch-${repo.id}`} class="w-full" value={branch}
    onchange={saveBranch} disabled={busy || !!pending} aria-describedby={`runtime-branch-help-${repo.id}`}>
    <NativeSelect.Option value="">Default branch</NativeSelect.Option>
    {#if branch && !branches.includes(branch)}
     <NativeSelect.Option value={branch}>{branch} (saved selection)</NativeSelect.Option>
    {/if}
    {#each branches as name (name)}<NativeSelect.Option value={name}>{name}</NativeSelect.Option>{/each}
   </NativeSelect.Root>
  </AutosaveField>
  <p id={`runtime-branch-help-${repo.id}`} class="text-xs text-muted-foreground">Check the selected branch before reviewing its commit.</p>
  {#if loadingBranches}<p role="status" class="text-xs text-muted-foreground">Loading branches…</p>
  {:else if branchError}
   <Feedback kind="error" title="Could not list branches" message={branchError}/>
   <Button variant="outline" onclick={() => loadBranches(nextBranchPage || 1)}>Retry branch list</Button>
  {:else if nextBranchPage}
   <Button variant="outline" onclick={() => loadBranches(nextBranchPage)}>More branches</Button>
  {/if}
 </div>
{/snippet}

{#snippet inspectionActivity()}
 <details bind:open={expanded} class="rounded-xl border p-4">
  <summary class="cursor-pointer text-sm font-medium">Inspect a specific commit</summary>
  <div class="space-y-4 pt-4">
   <p class="text-sm text-muted-foreground">Enter an exact commit SHA to check a specific revision. The runtime branch check uses the latest commit on the selected branch.</p>
   {#if error}<Feedback kind="error" title="Inspection needs attention" message={error}/>{/if}
   <div class="space-y-2"><Label for={`inspection-commit-${repo.id}`}>Commit SHA</Label><Input id={`inspection-commit-${repo.id}`} bind:value={commitSha} placeholder="Full 40-character lowercase commit SHA" disabled={busy || !!pending} maxlength={40}/></div>
   {#if !canInspect}<p class="text-sm">Enable and validate the repository's source connection before starting an inspection.</p>{/if}
   <div class="flex flex-wrap gap-2"><Button disabled={busy || !!pending || !canInspect || !validCommit} onclick={() => perform(inspect)}>Inspect commit</Button><Button variant="outline" disabled={busy} onclick={() => perform(() => load())}>Refresh inspections</Button></div>
  </div>
 </details>
 <details class="rounded-xl border p-4" data-inspection-history>
  <summary class="cursor-pointer text-sm font-medium">Inspection history ({inspections.length}{next ? '+' : ''})</summary>
  <div class="space-y-4 pt-4"><p class="text-sm text-muted-foreground">Saved checks are historical evidence. An older awaiting-retry check does not block the latest completed check.</p>
  {#each inspections as item (item.id)}
   <section class="space-y-3 rounded-md border p-4" aria-label={`Inspection ${item.commitSha}`}>
    <p class="font-medium">{item.state === 'completed' ? 'Static inspection complete' : item.state === 'accepted' ? 'Inspection awaiting retry' : item.state === 'superseded' ? 'Source connection changed' : 'Inspection failed'}</p>
    <p class="text-sm text-muted-foreground">Commit <code>{item.commitSha.slice(0,12)}</code> · {new Date(Number(item.updatedAtUnixMs)).toLocaleString()}</p>
    {#if item.safeReason}<p class="text-sm">{inspectionReasons[item.safeReason] || 'Source inspection needs attention. Check the source connection and retry.'}</p>{/if}
    <details class="rounded-md border p-3"><summary class="cursor-pointer text-sm font-medium">Inspection technical details</summary><div class="space-y-3 pt-3">
    <InformationTable rows={[
     { label: 'Commit', value: item.commitSha, monospace: true },
     { label: 'Operation', value: item.id, monospace: true },
     { label: 'Source connection revision', value: item.connectionRevision },
     { label: 'Updated', value: new Date(Number(item.updatedAtUnixMs)).toLocaleString() },
    ]} />
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
    {/if}
    </div></details>
    <details><summary class="cursor-pointer text-sm">Inspection findings</summary><div class="space-y-3 pt-3">    {#if item.result}
     {#each item.result.findings || [] as finding (finding.code + finding.path)}
      <div class="space-y-1 border-t pt-3"><p class="text-sm font-medium">{finding.path || 'Preparation requirements'}</p><p class="text-sm">{finding.action}</p><details><summary class="cursor-pointer text-xs text-muted-foreground">Finding code</summary><code class="text-xs text-muted-foreground">{finding.code}</code></details></div>
     {/each}
    {/if}
</div></details>
    {#if item.state === 'accepted'}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(async () => {persist({ idempotencyKey: item.idempotencyKey, repositoryId: item.repositoryId, expectedConnectionRevision: item.connectionRevision, commitSha: item.commitSha }); await resume();})}>Resume inspection</Button>{/if}

   </section>
  {/each}

   {#if next}<Button variant="outline" disabled={busy} onclick={() => perform(() => load(true))}>More inspections</Button>{/if}
  </div>
 </details>
{/snippet}

<div data-repository-inspection={repo.id}>
 <RuntimePreparation {repo} {connection} branch={branch.trim()} checkedCommit={checked?.branch === branch.trim() ? checked?.sha : undefined} latestInspection={inspections[0] || null} inspectionLoading={!inspectionLoaded && !error} inspectionError={error} checkingDefault={checkingDefault} recheckDisabled={busy || !!pending || !canInspect} inspectionPending={!!pending} refreshInspections={() => load()} recheckDefaultBranch={() => perform(recheckDefaultBranch)}>
  {#snippet children(runtime, preparationActivity)}
   <Tabs.Content value="runtime" class="space-y-6">
    {@render recovery()}
    {@render branchPicker()}
    {@render runtime()}
   </Tabs.Content>
   <Tabs.Content value="activity" class="space-y-6">
    {@render recovery()}
    {@render preparationActivity()}
    {@render inspectionActivity()}
   </Tabs.Content>
  {/snippet}
 </RuntimePreparation>
</div>
