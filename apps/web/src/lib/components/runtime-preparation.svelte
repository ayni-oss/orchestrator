<script lang="ts">
 import { protectUnsavedChanges } from "$lib/unsaved-changes.svelte";
 import { onMount, type Snippet } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';
 import RuntimeSignerManager from '$lib/components/runtime-signer-manager.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import { trustSnippet } from '$lib/runtime-signing';
 import { inspectionReasons, type Repository, type RepositoryInspection } from '$lib/repositories';
 import { runtimeOverview, runtimeStages, runtimeReasons, onboardingReasons, runtimeRegistryImage, type RepositoryRuntimeRun, type RuntimePreparation, type RuntimeSigner, type RuntimeCommand } from '$lib/runtime-preparation';
 let { repo, branch = '', checkedCommit, latestInspection = null, inspectionLoading = false, inspectionError = '', connection, recheckDefaultBranch, recheckDisabled = false, checkingDefault = false, inspectionPending = false, refreshInspections, children }: { repo: Repository; branch?: string; checkedCommit?: string; latestInspection?: RepositoryInspection | null; inspectionLoading?: boolean; inspectionError?: string; connection?: Connection; recheckDefaultBranch?: () => Promise<void>; recheckDisabled?: boolean; checkingDefault?: boolean; inspectionPending?: boolean; refreshInspections?: () => Promise<void>; children: Snippet<[Snippet, Snippet]> } = $props();
 let onboarding = $state.raw<RepositoryRuntimeRun | null>(null);
 let deploymentEnabled = $state(false), developmentOnly = $state(false);
 let loaded = $state(false);
 let integrationSetup = $state<'registry' | 'signer' | null>(null);
 let expanded = $state(false), busy = $state(false), error = $state('');
 let polling = false, mounted = false, historySequence = 0;
 let pending = $state<RuntimeCommand | null>(null);
 let rejected = $state(false);
 let registries = $state.raw<Connection[]>([]), signers = $state.raw<RuntimeSigner[]>([]), operations = $state.raw<RuntimePreparation[]>([]);
 let next = $state(''), registryId = $state(''), signerId = $state(''), platform = $state('linux/arm64');
 let seconds = $state(1800), builds = $state(1), allowBuild = $state(false), allowReplace = $state(false);
 let reviewedSettings = $state('');
 const settingsDraft = $derived(JSON.stringify({registryId,signerId,platform,seconds,builds,allowBuild,allowReplace}));
 protectUnsavedChanges(()=>expanded&&!pending&&!!reviewedSettings&&settingsDraft!==reviewedSettings);
 const overview = $derived(runtimeOverview({loaded,error,rejected,inspectionPending,moreHistory:!!next,pending:!!pending,inspectionLoading,inspectionError,inspectionReason:inspectionReasons[latestInspection?.safeReason || ''],inspection:latestInspection,operations,registries,signers,sourceReady:!!connection?.enabled && !!connection?.activeCredentialRevision,sourceRevision:connection?.revision}));
 const storage = $derived(`ayni.runtime.preparation.pending.${repo.id}`);
 const candidateInspection = $derived(latestInspection);
 const onboardingActive = $derived(!!onboarding && ['resolving','inspecting','admitting'].includes(onboarding.state));
 const onboardingBlocked = $derived(onboarding?.state === 'blocked');
 const profileRestored = $derived(onboardingBlocked && onboarding?.safeReason === 'runtime_profile_unavailable' && deploymentEnabled);
 const workActive = $derived(operations.some(value => activeStates.has(value.state) || value.state === 'publication_uncertain' || !value.cleanupComplete));
 const canConfigure = $derived(loaded && !busy && !pending && !inspectionPending && !checkingDefault && !onboardingActive && !workActive && !!connection?.enabled && !!connection?.activeCredentialRevision);
 const savedSettingsAvailable = $derived(!!onboarding?.settings && registries.some(value => value.id === onboarding?.settings.registryConnectionId && value.enabled && value.activeCredentialRevision) && signers.some(value => value.id === onboarding?.settings.signerId && value.enabled));
 const previousOperations = $derived(operations.filter(value => value.id !== overview.operation?.id));
 const registry = $derived(registries.find(value => value.id === registryId));
 const signer = $derived(signers.find(value => value.id === signerId));
 const trustedSigners = $derived(signers.filter(value => value.enabled && candidateInspection?.result?.trustKeyIds?.includes(value.keyId)));
 const signerTrustMissing = $derived(!!signer && candidateInspection?.state === 'completed' &&
  !!candidateInspection.result?.lockFingerprint && !candidateInspection.result.trustKeyIds?.includes(signer.keyId));
 const destination = $derived(registry?.registryDestination ? `${registry.registryDestination}/${repo.fullName.split('/').at(-1)?.toLowerCase()}` : registries.find(value => value.id === onboarding?.settings.registryConnectionId)?.registryDestination ? `${registries.find(value => value.id === onboarding?.settings.registryConnectionId)?.registryDestination}/${repo.fullName.split('/').at(-1)?.toLowerCase()}` : '');
 const image = $derived(runtimeRegistryImage(onboardingActive || onboardingBlocked ? undefined : overview.operation,destination));
 const onboardingSteps = ['Connect integrations', 'Check repository', 'Review and start', 'Prepare image', 'Verify and finish'];
 const currentStep = $derived.by(() => {
  const operation = overview.operation;
  if (onboardingActive) return onboarding?.state === 'admitting' ? 2 : 1;
  if (!onboardingBlocked && operation) {
   return operation.state === 'ready' || ['pull_final', 'verify_content', 'verify_compatibility', 'cleanup'].includes(operation.stage) ? 4 : 3;
  }
  if (!deploymentEnabled || !connection?.enabled || !connection.activeCredentialRevision ||
   !registries.some(value => value.enabled && value.activeCredentialRevision) ||
   !signers.some(value => value.enabled)) return 0;
  if (checkingDefault || !latestInspection || overview.issues.length) return 1;
  return 2;
 });
 const activeStates = new Set(['pending', 'running', 'stopping', 'cancelling', 'cleanup_pending']);
 const retryStates = new Set(['failed', 'blocked', 'unavailable', 'publication_uncertain']);
 const validAllowance = $derived(Number.isInteger(seconds) && seconds >= 1 && seconds <= 86400 && (!allowBuild || Number.isInteger(builds) && builds >= 1 && builds <= 10));
 const reviewedCommit = $derived(latestInspection?.state === 'completed' && latestInspection.commitSha === checkedCommit && latestInspection.connectionRevision === connection?.revision ? checkedCommit : undefined);
 const canStart = $derived(!!connection?.enabled && !!connection.activeCredentialRevision && !onboardingActive && !workActive && !!registry?.enabled && !!registry?.activeCredentialRevision && !!signer?.enabled && validAllowance && !pending && !busy && (!branch || !!reviewedCommit));
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
 async function loadOnboarding() {
  const result = await rpc('GetRepositoryRuntime',{repositoryId:repo.id});
  if (!mounted) return;
  const changed = result.run && (result.run.id !== onboarding?.id || result.run.state !== onboarding?.state);
  onboarding = result.run || null;
  deploymentEnabled = result.preparationEnabled === true; developmentOnly = result.developmentOnly === true;
  if (changed && onboarding?.inspectionId) await refreshInspections?.();
 }
 async function refresh() { await Promise.all([configuration(), loadOnboarding()]); await history(); loaded = true; }
 async function open() { expanded = true; await refresh(); }
 function restoreSettings() {
  if (!onboarding?.settings) return;
  const saved=onboarding.settings;
  registryId=saved.registryConnectionId; signerId=saved.signerId; platform=saved.platform;
  seconds=saved.maxSeconds; builds=saved.maxBuilds || 1; allowBuild=saved.allowBuild; allowReplace=saved.allowReplace;
 }
 async function configure() {
  await open();
  allowBuild=false; allowReplace=false; builds=1;
  restoreSettings();
  const availableRegistries=registries.filter(value=>value.enabled && value.activeCredentialRevision);
  const availableSigners=signers.filter(value=>value.enabled);
  if (!registryId && availableRegistries.length===1) registryId=availableRegistries[0].id;
  if (!signerId && trustedSigners.length === 1) signerId = trustedSigners[0].id;
  if (!signerId && availableSigners.length===1) signerId=availableSigners[0].id;
  if (!availableRegistries.length) integrationSetup = 'registry';
  else if (!availableSigners.length) integrationSetup = 'signer';
  reviewedSettings=settingsDraft;
 }
 async function prepareLatest() { restoreSettings(); await start(); }

 async function accept(operation: RuntimePreparation) {
  if (operation.repositoryId !== repo.id) throw new Error('The retained preparation belongs to another repository.');
  persist(null);
  const current = await rpc('GetRuntimePreparation', { id: operation.id });
  operations = [current.operation, ...operations.filter(value => value.id !== operation.id)]; expanded = false; await history();
 }
 async function resume() {
  if (!pending) return;
  if (pending.method === 'StartRepositoryRuntime') {
   try { await rpc(pending.method,pending.request); persist(null); expanded=false; await refresh(); }
   catch (cause) { if (cause instanceof RpcError && [400,403,409].includes(cause.status)) rejected=true; throw cause; }
   return;
  }
  if (pending.method !== 'StartRuntimePreparation') {
   const current = await rpc('GetRuntimePreparation', { id: pending.request.id });
   if (current.operation?.repositoryId !== repo.id) throw new Error('The retained command belongs to another repository.');
  }
  try { const result = await rpc(pending.method, pending.request); await accept(result.operation); }
  catch (cause) { if (cause instanceof RpcError && [400, 403, 409].includes(cause.status)) rejected = true; throw cause; }
 }
 async function recover() { if (pending?.method === 'StartRepositoryRuntime') { await rpc('GetRepositoryRuntimeReceipt',{idempotencyKey:pending.request.idempotencyKey}); persist(null); expanded=false; await refresh(); return; } if (pending) { const result = await rpc('GetRuntimePreparationReceipt', { idempotencyKey: pending.request.idempotencyKey }); await accept(result.operation); } }
 async function start() {
  if (!connection || pending || !registry?.enabled || !registry.activeCredentialRevision || !signer?.enabled || !validAllowance || onboardingActive || workActive || (branch && !reviewedCommit)) return;
  persist({method:'StartRepositoryRuntime',request:{idempotencyKey:crypto.randomUUID(),repositoryId:repo.id,expectedConnectionRevision:connection.revision,registryConnectionId:registryId,signerId,platform,maxSeconds:seconds,maxBuilds:allowBuild ? builds : 0,allowBuild,allowReplace:allowBuild && allowReplace,...(reviewedCommit ? {branch,expectedCommitSha:reviewedCommit,expectedRegistryRevision:registry.revision,expectedSignerRevision:signer.revision} : {})}});
  await resume();
 }
 async function command(method: 'CancelRuntimePreparation' | 'RetryRuntimePreparation', operation: RuntimePreparation) {
  if (pending) return;
  persist({ method, request: { idempotencyKey: crypto.randomUUID(), id: operation.id, expectedGeneration: operation.generation } }); await resume();
 }
 async function poll() {
  if (!loaded || busy || polling || (!onboardingActive && !operations.some(value => activeStates.has(value.state)))) return;
  polling = true;
  try { await loadOnboarding(); await history(); } catch { if (mounted) error = 'Live updates paused. Refresh history to reconnect; accepted work remains retained.'; } finally { polling = false; }
 }
 onMount(() => {
  mounted = true;
  const saved = sessionStorage.getItem(storage);
  if (saved) {
   try { const value = JSON.parse(saved); if (!['StartRepositoryRuntime', 'StartRuntimePreparation', 'CancelRuntimePreparation', 'RetryRuntimePreparation'].includes(value.method) || typeof value.request?.idempotencyKey !== 'string' || ['StartRepositoryRuntime','StartRuntimePreparation'].includes(value.method) && value.request.repositoryId !== repo.id) throw new Error(); pending = value; expanded = true; void perform(open); }
   catch { error = 'The saved preparation command could not be read. Refresh retained history before starting another.'; expanded = true; }
  } else { void perform(refresh); }
  const timer = setInterval(() => void poll(), 2000);
  return () => { mounted = false; clearInterval(timer); };
 });
</script>
{#snippet preparationEvidence(operation: RuntimePreparation)}
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
{/snippet}
{#snippet runtime()}
<section class="space-y-5" data-runtime-preparation={repo.id} aria-label={`Runtime preparation for ${repo.fullName}`}>
 <section class="space-y-4 rounded-xl border bg-card p-5" aria-label="Runtime status" aria-live="polite">
  {#if loaded}
   <ol aria-label="Runtime onboarding steps" class="grid gap-2 sm:grid-cols-5">
    {#each onboardingSteps as step, index (step)}
     <li aria-current={index === currentStep ? 'step' : undefined} class="rounded-md border px-2 py-1.5 text-xs" class:font-semibold={index === currentStep} class:bg-muted={index === currentStep}>
      <span class="tabular-nums">{index}.</span> {step}
     </li>
    {/each}
   </ol>
  {/if}
  <div class="space-y-2"><h3 class="text-lg font-semibold">{onboardingActive ? 'Preparing repository runtime' : profileRestored ? 'Ready to prepare runtime' : onboardingBlocked ? 'Runtime onboarding needs attention' : checkingDefault ? 'Checking branch' : overview.title}</h3><p class="text-sm text-muted-foreground">{onboardingActive ? ({resolving:'Checking the branch head…',inspecting:'Checking the committed Ayni policy, lock and build inputs…',admitting:'Validating build capability, signer trust and preparation settings…'}[onboarding?.state || ''] || 'Preparing…') : profileRestored ? 'Runtime building is now enabled. Prepare the latest commit to start a new run.' : onboardingBlocked ? onboardingReasons[onboarding?.safeReason || ''] || 'The runtime setup needs attention. Review the current configuration.' : checkingDefault ? 'Resolving the selected branch and inspecting its latest commit…' : overview.description}</p></div>
  {#if candidateInspection}<p class="text-xs text-muted-foreground">Last check: {new Date(Number(candidateInspection.updatedAtUnixMs)).toLocaleString()}{candidateInspection.result?.ayniVersion ? ` · Ayni ${candidateInspection.result.ayniVersion}` : ''}</p>{/if}
  {#if overview.issues.length && !checkingDefault && !onboardingActive}<div class="space-y-2"><h4 class="font-medium">What needs attention</h4><ul class="list-disc space-y-2 break-words pl-5 text-sm">{#each overview.issues as issue (issue)}<li>{issue}</li>{/each}</ul></div>{/if}
  {#if developmentOnly || overview.operation?.developmentOnly}<p class="rounded-md border p-3 text-sm">Local development build mode. Cluster network isolation is not qualified; this result is for local development testing.</p>{/if}
  {#if loaded && !deploymentEnabled && currentStep === 0}<p class="text-sm">Ask your administrator to enable a Kubernetes runtime profile.</p>{/if}
  {#if overview.nextAction && !checkingDefault && !onboardingActive && !onboardingBlocked}<p class="text-sm"><span class="font-medium">Next step:</span> {expanded && !pending ? 'Review the configuration below, then prepare the latest commit on the selected branch.' : canConfigure ? 'Prepare the latest commit using the reviewed registry and signing settings.' : overview.nextAction}</p>{/if}
  <div class="flex flex-wrap gap-2">
   {#if canConfigure && !expanded}{#if onboarding?.settings}<Button disabled={!savedSettingsAvailable || (!!branch && !reviewedCommit)} onclick={() => perform(prepareLatest)}>Prepare latest commit</Button><Button variant="outline" onclick={() => perform(configure)}>Change runtime settings</Button>{:else}<Button onclick={() => perform(configure)}>Configure runtime</Button>{/if}{/if}
   {#if overview.operation && retryStates.has(overview.operation.state)}{@const current = overview.operation}<Button disabled={busy || !!pending} onclick={() => perform(() => command('RetryRuntimePreparation',current))}>{current.state === 'publication_uncertain' ? 'Reconcile publication' : 'Retry preparation'}</Button>{/if}
   {#if overview.operation && activeStates.has(overview.operation.state) && overview.operation.state !== 'cancelling'}{@const current = overview.operation}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(() => command('CancelRuntimePreparation',current))}>Cancel preparation</Button>{/if}
   <Button variant={canConfigure || !!overview.operation ? 'outline' : 'default'} disabled={busy || recheckDisabled || checkingDefault || !!pending || onboardingActive} onclick={() => perform(async () => { await recheckDefaultBranch?.(); await refresh(); })}>{branch ? 'Check branch' : 'Recheck default branch'}</Button>
  </div>
  {#if loaded && !registries.some(value => value.enabled && value.activeCredentialRevision)}<Button variant="link" href="/settings/container-registries">Configure a runtime registry</Button>{/if}
  {#if loaded && !signers.some(value => value.enabled)}<Button variant="link" href="/settings/runtime-signing">Add a signing key</Button>{/if}
  {#if onboarding?.settings && !savedSettingsAvailable && !onboardingActive}<p class="text-sm">The saved registry or signing key is unavailable. Change runtime settings to choose enabled connections.</p>{/if}
  {#if onboarding?.settings && !expanded}<p class="text-xs text-muted-foreground">Reviewed allowance: {onboarding.settings.allowBuild ? `up to ${onboarding.settings.maxBuilds} build(s)` : 'existing images only'} · {onboarding.settings.maxSeconds} seconds · {onboarding.settings.platform}. Each preparation is a separate bounded run.</p>{/if}
  {#if image}<div class="space-y-2 border-t pt-3"><p class="text-sm font-medium">{image.confirmed ? 'Registry image available' : 'Image destination'}</p><code class="block break-all text-xs">{image.reference || image.destination}</code>{#if !image.confirmed}<p class="text-xs text-muted-foreground">No published image is confirmed by this status yet.</p>{/if}{#if image.reference && overview.operation?.state === 'ready'}
   {@const reference = image.reference}
   <Button variant="outline" onclick={() => perform(() => navigator.clipboard.writeText(reference))}>Copy image reference</Button>
  {/if}<a class="text-sm text-primary underline underline-offset-4" href={image.url} target="_blank" rel="noreferrer">Open registry package</a></div>{/if}
  <p class="text-xs text-muted-foreground">Prepare latest commit runs the complete pipeline using the reviewed settings. Recheck only inspects source. Runtime readiness applies to the exact commit and platform shown.</p>
 </section>
 {#if error && !pending}<Feedback kind="error" title="Preparation needs attention" message={error}/>{/if}
 {#if pending}<section class="space-y-3 rounded-xl border p-5" aria-label="Current preparation recovery"><h3 class="font-semibold">Confirm the last preparation command</h3><p class="text-sm">The response was interrupted. Recover the saved receipt before starting another command.</p>{#if error}<p class="text-sm" role="alert">{error}</p>{/if}<div class="flex flex-wrap gap-2"><Button disabled={busy} onclick={() => perform(recover)}>Recover preparation receipt</Button><Button variant="outline" disabled={busy} onclick={() => perform(resume)}>Retry same preparation command</Button>{#if rejected}<Button variant="outline" disabled={busy} onclick={() => {persist(null);error='';}}>Dismiss rejected command</Button>{/if}</div></section>{/if}
 {#if expanded && !onboardingActive && !pending}
  <section class="space-y-4 rounded-xl border p-5" aria-label="Configure current runtime">
   <h3 class="font-semibold">Prepare and publish the repository runtime</h3>
   {#if reviewedCommit}
    <p class="text-sm">Reviewed commit <code class="break-all">{reviewedCommit}</code> on {branch || 'the default branch'}.</p>
   {:else if branch}
    <p class="text-sm">Check this branch before starting preparation.</p>
   {/if}
   <p class="text-sm text-muted-foreground">Orchestrator checks the selected branch and Ayni setup, then verifies an existing image or builds, certifies and publishes one within the allowance below. This run continues if you close the browser.</p>

   {#if integrationSetup === 'registry'}
    <div class="space-y-3 rounded-md border p-3">
     <p class="text-sm">Connect a registry so Orchestrator can reuse or publish runtime images.</p>
     <ProviderConnectionManager capability="container_registry" createOnMount onSaved={value => { registries = [value, ...registries.filter(item => item.id !== value.id)]; registryId = value.id; integrationSetup = signers.some(item => item.enabled) ? null : 'signer'; }} />
     <Button variant="outline" onclick={() => { integrationSetup = null; }}>Close registry setup</Button>
    </div>
   {:else if integrationSetup === 'signer'}
    <div class="space-y-3 rounded-md border p-3">
     <p class="text-sm">A signing key certifies images. The repository uses its public key to verify the runtime.</p>
     <RuntimeSignerManager createOnMount onSaved={value => { signers = [value, ...signers.filter(item => item.id !== value.id)]; signerId = value.id; integrationSetup = null; }} />
     <Button variant="outline" onclick={() => { integrationSetup = null; }}>Close signing setup</Button>
    </div>
   {/if}
   <div class="space-y-2"><Label for={`runtime-registry-${repo.id}`}>Runtime registry</Label><NativeSelect.Root id={`runtime-registry-${repo.id}`} bind:value={registryId} disabled={busy || !!pending}><NativeSelect.Option value="">Choose a registry connection</NativeSelect.Option>{#each registries as value (value.id)}<NativeSelect.Option value={value.id} disabled={!value.enabled || !value.activeCredentialRevision}>{value.displayName} · {value.registryDestination}{!value.enabled ? ' (disabled)' : !value.activeCredentialRevision ? ' (validation required)' : ''}</NativeSelect.Option>{/each}</NativeSelect.Root></div>
   <div class="space-y-2"><Label for={`runtime-signer-${repo.id}`}>Runtime signer</Label><NativeSelect.Root id={`runtime-signer-${repo.id}`} bind:value={signerId} disabled={busy || !!pending}><NativeSelect.Option value="">Choose a signer</NativeSelect.Option>{#each signers as value (value.id)}<NativeSelect.Option value={value.id} disabled={!value.enabled}>{value.keyId}{!value.enabled ? ' (disabled)' : ''}</NativeSelect.Option>{/each}</NativeSelect.Root></div>
   {#if signer}<details class="rounded-md border p-3"><summary class="cursor-pointer text-sm font-medium">Signing key details</summary><div class="space-y-3 pt-3"><InformationTable rows={[{label:'Key name',value:signer.keyId},{label:'Public key',value:signer.publicKey || 'Unavailable',monospace:true}]}/><Button variant="outline" onclick={() => perform(() => navigator.clipboard.writeText(signer!.publicKey))}>Copy public key</Button><p class="text-xs text-muted-foreground">Use this key name and public key together in the repository’s .ayni.toml trust policy.</p></div></details>{/if}
   {#if signer && signerTrustMissing}
    <div class="space-y-2 rounded-md border p-3">
     <p class="text-sm font-medium">The checked commit does not trust this signing key.</p>
     {#if trustedSigners.length}
      <p class="text-sm">Choose an enabled key already named in the repository policy.</p>
      {#each trustedSigners as trusted (trusted.id)}
       <Button variant="outline" onclick={() => { signerId = trusted.id; }}>Use {trusted.keyId}</Button>
      {/each}
     {/if}
     <p class="text-sm">To use the selected key, add its public entry to the existing trust policy in <code>.ayni.toml</code> and preserve the other keys. Run <code>ayni env lock</code>, commit both files, then recheck the branch.</p>
     <pre class="overflow-x-auto rounded-md bg-muted p-2 text-xs">{trustSnippet(signer)}</pre>
    </div>
   {/if}
   <div class="flex flex-wrap gap-4"><div class="space-y-2"><Label for={`runtime-platform-${repo.id}`}>Runtime platform</Label><NativeSelect.Root id={`runtime-platform-${repo.id}`} bind:value={platform} disabled={busy || !!pending}><NativeSelect.Option value="linux/arm64">Linux arm64</NativeSelect.Option><NativeSelect.Option value="linux/amd64">Linux amd64</NativeSelect.Option></NativeSelect.Root></div><div class="space-y-2"><Label for={`runtime-seconds-${repo.id}`}>Time limit (seconds)</Label><Input id={`runtime-seconds-${repo.id}`} type="number" min={1} max={86400} bind:value={seconds} disabled={busy || !!pending}/></div></div>
   <label class="flex items-start gap-2 text-sm"><input type="checkbox" bind:checked={allowBuild} onchange={() => { if (!allowBuild) allowReplace = false; }} disabled={busy || !!pending}/>Allow a bounded build if the image is missing</label>
   {#if allowBuild}<div class="space-y-2"><Label for={`runtime-builds-${repo.id}`}>Maximum build attempts</Label><Input id={`runtime-builds-${repo.id}`} type="number" min={1} max={10} bind:value={builds} disabled={busy || !!pending}/><p class="text-xs text-muted-foreground">The qualified deployment resource limits also apply. Interrupted builds consume their retained attempt allowance.</p></div><label class="flex items-start gap-2 text-sm"><input type="checkbox" bind:checked={allowReplace} disabled={busy || !!pending}/>Also allow replacement if an existing image is invalid</label>{/if}
   <div class="flex flex-wrap gap-2"><Button disabled={!canStart} onclick={() => perform(start)}>{allowBuild ? 'Build and publish runtime' : 'Check and prepare runtime'}</Button><Button variant="outline" disabled={busy || !!pending} onclick={() => {expanded=false;}}>Close setup</Button><Button variant="outline" onclick={() => { integrationSetup = 'registry'; }}>Add registry</Button><Button variant="outline" onclick={() => { integrationSetup = 'signer'; }}>Add signing key</Button></div>
  </section>

 {/if}
</section>
{/snippet}

{#snippet preparationActivity()}
 {#if error && !pending}<Feedback kind="error" title="Preparation needs attention" message={error}/>{/if}
 {#if overview.operation}<details class="rounded-xl border p-4"><summary class="cursor-pointer text-sm font-medium">Current preparation details</summary><div class="pt-4">{@render preparationEvidence(overview.operation)}</div></details>{/if}
 <details class="rounded-xl border p-4" data-preparation-history>
  <summary class="cursor-pointer text-sm font-medium">Previous preparations ({previousOperations.length}{next ? '+' : ''})</summary>
  <div class="space-y-4 pt-4"><Button variant="outline" disabled={busy} onclick={() => perform(refresh)}>Refresh preparation history</Button>
   {#if !previousOperations.length}<p class="text-sm text-muted-foreground">No earlier runtime preparations.</p>{/if}
   {#each previousOperations as operation (operation.id)}{@render preparationEvidence(operation)}{/each}
   {#if next}<Button variant="outline" disabled={busy} onclick={() => perform(() => history(true))}>More preparations</Button>{/if}
  </div>
 </details>
{/snippet}

{@render children(runtime, preparationActivity)}
