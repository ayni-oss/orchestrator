<script lang="ts">
 import { onMount } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import Feedback from '$lib/components/feedback.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import type { Repository, RepositoryInspection } from '$lib/repositories';
 import { runtimeStages, runtimeReasons, type RuntimePreparation, type RuntimeSigner, type RuntimeCommand } from '$lib/runtime-preparation';
 let { repo, inspection = null }: { repo: Repository; inspection?: RepositoryInspection | null } = $props();
 let expanded = $state(false), busy = $state(false), error = $state('');
 let polling = false, mounted = false, historySequence = 0;
 let pending = $state<RuntimeCommand | null>(null);
 let rejected = $state(false);
 let registries = $state.raw<Connection[]>([]), signers = $state.raw<RuntimeSigner[]>([]), operations = $state.raw<RuntimePreparation[]>([]);
 let next = $state(''), registryId = $state(''), signerId = $state(''), platform = $state('linux/arm64');
 let seconds = $state(1800), builds = $state(1), allowBuild = $state(false), allowReplace = $state(false);
 const storage = $derived(`ayni.runtime.preparation.pending.${repo.id}`);
 const usableInspection = $derived(inspection?.state === 'completed' && !!inspection.result && !(inspection.result.findings || []).some(f => !['adapter_discovery_pending', 'runtime_verification_pending'].includes(f.code)));
 const registry = $derived(registries.find(value => value.id === registryId));
 const signer = $derived(signers.find(value => value.id === signerId));
 const activeStates = new Set(['pending', 'running', 'stopping', 'cancelling', 'cleanup_pending']);
 const retryStates = new Set(['failed', 'blocked', 'unavailable', 'publication_uncertain']);
 const validAllowance = $derived(Number.isInteger(seconds) && seconds >= 1 && seconds <= 86400 && (!allowBuild || Number.isInteger(builds) && builds >= 1 && builds <= 10));
 const canStart = $derived(usableInspection && !!registry?.enabled && !!registry?.activeCredentialRevision && !!signer?.enabled && validAllowance && !pending && !busy);
 function persist(value: RuntimeCommand | null) { pending = value; rejected = false; if (value) sessionStorage.setItem(storage, JSON.stringify(value)); else sessionStorage.removeItem(storage); }
 async function perform(action: () => Promise<void>) {
  busy = true; error = '';
  try { await action(); } catch (cause) { error = cause instanceof RpcError && cause.status === 409 ? 'The preparation generation or selected configuration changed. Recover the retained command and refresh history before another action.' : cause instanceof Error ? cause.message : 'Preparation could not be loaded. Recover the retained command or refresh history.'; } finally { busy = false; }
 }
 async function history(append = false) {
  const sequence = ++historySequence;
  const result = await rpc('ListRuntimePreparations', { repositoryId: repo.id, afterId: append ? next : '', limit: 20 });
  if (!mounted || sequence !== historySequence) return;
  operations = append ? [...operations, ...(result.operations || [])] : result.operations || []; next = result.nextAfterId || '';
 }
 async function configuration() {
  const connections: Connection[] = [], identities: RuntimeSigner[] = [];
  let afterId = '', pages = 0;
  do { const page = await rpc('ListConnections', { afterId, limit: 100 }); connections.push(...(page.connections || []).filter((value: Connection) => value.capability === 'container_registry')); afterId = page.nextAfterId || ''; } while (afterId && ++pages < 20);
  afterId = ''; pages = 0;
  do { const page = await rpc('ListRuntimeSigners', { afterId, limit: 100 }); identities.push(...(page.signers || [])); afterId = page.nextAfterId || ''; } while (afterId && ++pages < 20);
  if (mounted) { registries = connections; signers = identities; }
 }
 async function open() { expanded = true; await Promise.all([configuration(), history()]); }
 async function accept(operation: RuntimePreparation) {
  if (operation.repositoryId !== repo.id) throw new Error('The retained preparation belongs to another repository.');
  persist(null);
  const current = await rpc('GetRuntimePreparation', { id: operation.id });
  operations = [current.operation, ...operations.filter(value => value.id !== operation.id)]; await history();
 }
 async function resume() {
  if (!pending) return;
  if (pending.method !== 'StartRuntimePreparation') {
   const current = await rpc('GetRuntimePreparation', { id: pending.request.id });
   if (current.operation?.repositoryId !== repo.id) throw new Error('The retained command belongs to another repository.');
  }
  try { const result = await rpc(pending.method, pending.request); await accept(result.operation); }
  catch (cause) { if (cause instanceof RpcError && [400, 403, 409].includes(cause.status)) rejected = true; throw cause; }
 }
 async function recover() { if (pending) { const result = await rpc('GetRuntimePreparationReceipt', { idempotencyKey: pending.request.idempotencyKey }); await accept(result.operation); } }
 async function start() {
  if (!usableInspection || !inspection || pending || !registry?.enabled || !registry.activeCredentialRevision || !signer?.enabled || !validAllowance) return;
  persist({ method: 'StartRuntimePreparation', request: { idempotencyKey: crypto.randomUUID(), repositoryId: repo.id, inspectionId: inspection.id, registryConnectionId: registryId, signerId, platform, maxSeconds: seconds, maxBuilds: allowBuild ? builds : 0, allowBuild, allowReplace: allowBuild && allowReplace } }); await resume();
 }
 async function command(method: 'CancelRuntimePreparation' | 'RetryRuntimePreparation', operation: RuntimePreparation) {
  if (pending) return;
  persist({ method, request: { idempotencyKey: crypto.randomUUID(), id: operation.id, expectedGeneration: operation.generation } }); await resume();
 }
 async function poll() {
  if (!expanded || busy || polling || !operations.some(value => activeStates.has(value.state))) return;
  polling = true;
  try { await history(); } catch { if (mounted) error = 'Live updates paused. Refresh history to reconnect; accepted work remains retained.'; } finally { polling = false; }
 }
 onMount(() => {
  mounted = true;
  const saved = sessionStorage.getItem(storage);
  if (saved) {
   try { const value = JSON.parse(saved); if (!['StartRuntimePreparation', 'CancelRuntimePreparation', 'RetryRuntimePreparation'].includes(value.method) || typeof value.request?.idempotencyKey !== 'string' || value.method === 'StartRuntimePreparation' && value.request.repositoryId !== repo.id) throw new Error(); pending = value; expanded = true; void perform(open); }
   catch { error = 'The saved preparation command could not be read. Refresh retained history before starting another.'; expanded = true; }
  }
  const timer = setInterval(() => void poll(), 2000);
  return () => { mounted = false; clearInterval(timer); };
 });
</script>
<section class="space-y-4 border-t pt-4" data-runtime-preparation={repo.id} aria-label={`Runtime preparation for ${repo.fullName}`}>
 <h3 class="font-semibold">Runtime preparation</h3>
 <p class="text-sm text-muted-foreground">Verify an immutable runtime for an inspected commit. Building and replacement use explicit allowances. Preparation starts no tasks.</p>
 {#if inspection}<p class="break-all text-sm">Selected commit: <code>{inspection.commitSha}</code></p>{:else}<p class="text-sm">Select a completed inspection above to prepare its exact commit.</p>{/if}
 {#if !expanded}<Button variant="outline" disabled={busy} onclick={() => perform(open)}>Open runtime preparation</Button>{:else}
  {#if error}<Feedback kind="error" title="Preparation needs attention" message={error}/>{/if}
  {#if busy}<p role="status" class="text-sm">Loading runtime preparation…</p>{/if}
  {#if pending}<div class="space-y-3 rounded-md border p-4"><p class="font-medium">Recover preparation command</p><code class="break-all text-xs">{pending.request.idempotencyKey}</code><p class="text-sm">Recover the receipt or retry the exact command after an interrupted response.</p><div class="flex flex-wrap gap-2"><Button disabled={busy} onclick={() => perform(recover)}>Recover preparation receipt</Button><Button variant="outline" disabled={busy} onclick={() => perform(resume)}>Retry same preparation command</Button></div></div>{/if}
  {#if pending && rejected}<Button variant="outline" disabled={busy} onclick={() => { persist(null); error = ''; }}>Dismiss rejected command</Button>{/if}
  <div class="space-y-4 rounded-md border p-4">
   {#if !usableInspection}<p class="text-sm">Select an inspection whose policy, lock, input and trust findings are resolved.</p>{/if}
   <div class="space-y-2"><Label for={`runtime-registry-${repo.id}`}>Runtime registry</Label><NativeSelect.Root id={`runtime-registry-${repo.id}`} bind:value={registryId} disabled={busy || !!pending}><NativeSelect.Option value="">Choose a registry connection</NativeSelect.Option>{#each registries as value (value.id)}<NativeSelect.Option value={value.id} disabled={!value.enabled || !value.activeCredentialRevision}>{value.displayName} · {value.registryDestination}{!value.enabled ? ' (disabled)' : !value.activeCredentialRevision ? ' (validation required)' : ''}</NativeSelect.Option>{/each}</NativeSelect.Root></div>
   <div class="space-y-2"><Label for={`runtime-signer-${repo.id}`}>Runtime signer</Label><NativeSelect.Root id={`runtime-signer-${repo.id}`} bind:value={signerId} disabled={busy || !!pending}><NativeSelect.Option value="">Choose a signer</NativeSelect.Option>{#each signers as value (value.id)}<NativeSelect.Option value={value.id} disabled={!value.enabled || !!inspection?.result?.trustKeyIds && !inspection.result.trustKeyIds.includes(value.keyId)}>{value.displayName} · {value.keyId}{!value.enabled ? ' (disabled)' : ''}</NativeSelect.Option>{/each}</NativeSelect.Root></div>
   <div class="flex flex-wrap gap-4"><div class="space-y-2"><Label for={`runtime-platform-${repo.id}`}>Runtime platform</Label><NativeSelect.Root id={`runtime-platform-${repo.id}`} bind:value={platform} disabled={busy || !!pending}><NativeSelect.Option value="linux/arm64">Linux arm64</NativeSelect.Option><NativeSelect.Option value="linux/amd64">Linux amd64</NativeSelect.Option></NativeSelect.Root></div><div class="space-y-2"><Label for={`runtime-seconds-${repo.id}`}>Time limit (seconds)</Label><Input id={`runtime-seconds-${repo.id}`} type="number" min={1} max={86400} bind:value={seconds} disabled={busy || !!pending}/></div></div>
   <label class="flex items-start gap-2 text-sm"><input type="checkbox" bind:checked={allowBuild} onchange={() => { if (!allowBuild) allowReplace = false; }} disabled={busy || !!pending}/>Allow a bounded build if the image is missing</label>
   {#if allowBuild}<div class="space-y-2"><Label for={`runtime-builds-${repo.id}`}>Maximum build attempts</Label><Input id={`runtime-builds-${repo.id}`} type="number" min={1} max={10} bind:value={builds} disabled={busy || !!pending}/><p class="text-xs text-muted-foreground">The qualified deployment resource limits also apply. Interrupted builds consume their retained attempt allowance.</p></div><label class="flex items-start gap-2 text-sm"><input type="checkbox" bind:checked={allowReplace} disabled={busy || !!pending}/>Also allow replacement if an existing image is invalid</label>{/if}
   <div class="flex flex-wrap gap-2"><Button disabled={!canStart} onclick={() => perform(start)}>{allowBuild ? 'Verify or prepare runtime' : 'Verify existing runtime'}</Button><Button variant="outline" disabled={busy} onclick={() => perform(open)}>Refresh preparation history</Button><Button variant="link" href="/settings/container-registries">Manage registries</Button><Button variant="link" href="/settings/runtime-signing">Manage signers</Button></div>
  </div>
  <h4 class="font-medium">Preparation history</h4>
  {#if !operations.length && !busy}<p class="text-sm text-muted-foreground">No runtime preparations for this repository.</p>{/if}
  {#each operations as operation (operation.id)}
   <article class="space-y-3 rounded-md border p-4" aria-label={`Runtime preparation ${operation.id}`}>
    <p class="font-medium">{operation.state === 'ready' ? 'Runtime ready' : operation.state.replaceAll('_', ' ')} · {runtimeStages[operation.stage] || operation.stage}</p>
    <InformationTable rows={[
     {label:'Platform',value:operation.platform},
     {label:'Content verification',value:operation.contentVerified ? 'Verified' : 'Not verified'},
     {label:'Managed compatibility',value:operation.managedCompatible ? 'Verified' : 'Not verified'},
     {label:'Cleanup',value:operation.cleanupComplete ? 'Complete' : 'Not complete'},
    ]}/>
    <details class="rounded-md border p-3"><summary class="cursor-pointer text-sm font-medium">Preparation technical details</summary><div class="pt-3">
    <InformationTable rows={[
     {label:'Operation',value:operation.id,monospace:true},{label:'Inspection',value:operation.inspectionId,monospace:true},{label:'Generation',value:operation.generation},
     {label:'Frozen input digest',value:operation.frozenInputDigest,monospace:true},
     {label:'Source commit',value:operation.sourceCommit || 'Not retained',monospace:true},
     {label:'Source tree',value:operation.sourceTree || 'Not retained',monospace:true},
     {label:'Policy digest',value:operation.policyDigest || 'Not retained',monospace:true},
     {label:'Lock digest',value:operation.lockDigest || 'Not retained',monospace:true},
     {label:'Lock fingerprint',value:operation.lockFingerprint || 'Not retained',monospace:true},
     {label:'Registry destination',value:operation.registryDestination || 'Not retained'},
     {label:'Builds started / allowance',value:`${operation.buildsStarted || 0} / ${operation.maxBuilds || 0}`},
     {label:'Image digest',value:operation.imageDigest || 'Not retained',monospace:true},
     {label:'Publication intent',value:operation.expectedManifestDigest || 'Not retained',monospace:true},
     {label:'Protected content root',value:operation.protectedContentRoot || 'Not retained',monospace:true},
     {label:'Certificate digest',value:operation.certificateDigest || 'Not retained',monospace:true},
     {label:'Unsigned manifest digest',value:operation.unsignedManifestDigest || 'Not retained',monospace:true},
     {label:'Certification layer digest',value:operation.certificationLayerDigest || 'Not retained',monospace:true},
     {label:'Managed check digest',value:operation.managedCheckDigest || 'Not retained',monospace:true},
     {label:'Updated',value:new Date(Number(operation.updatedAtUnixMs)).toLocaleString()},
    ]}/>
    </div></details>
    {#if operation.safeReason}<p class="text-sm">{runtimeReasons[operation.safeReason] || 'This stage needs attention. Review the retained reason and configuration before retrying.'}</p><code class="text-xs">{operation.safeReason}</code>{/if}
    <div class="flex flex-wrap gap-2">{#if activeStates.has(operation.state) && operation.state !== 'cancelling'}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(() => command('CancelRuntimePreparation', operation))}>Cancel preparation</Button>{/if}{#if retryStates.has(operation.state)}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(() => command('RetryRuntimePreparation', operation))}>{operation.state === 'publication_uncertain' ? 'Reconcile publication' : 'Retry preparation'}</Button>{/if}</div>
   </article>
  {/each}
  {#if next}<Button variant="outline" disabled={busy} onclick={() => perform(() => history(true))}>More preparations</Button>{/if}
 {/if}
</section>
