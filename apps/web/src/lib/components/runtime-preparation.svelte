<script lang="ts">
 import { protectUnsavedChanges } from "$lib/unsaved-changes.svelte";
 import { onMount, tick, untrack, type Snippet } from 'svelte';
 import { RefreshCw } from '@lucide/svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import { toast } from 'svelte-sonner';
 import AutosaveField from '$lib/components/autosave-field.svelte';
 import InformationTable from '$lib/components/information-table.svelte';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import { trustSnippet } from '$lib/runtime-signing';
 import { inspectionReasons, type Repository, type RepositoryInspection } from '$lib/repositories';
 import { runtimeOverview, runtimeStages, runtimeReasons, onboardingReasons, runtimeRegistryImage, runtimeProgressKey, readRuntimeProgress, type RepositoryRuntimeRun, type RuntimePreparation, type RuntimeSigner, type RuntimeCommand } from '$lib/runtime-preparation';
 let { repo, branch = '', checkedCommit, latestInspection = null, inspectionLoading = false, inspectionError = '', connection, recheckDefaultBranch, recheckDisabled = false, checkingDefault = false, inspectionPending = false, refreshInspections, onSourceRefresh, branchPicker, children }: { repo: Repository; branch?: string; checkedCommit?: string; latestInspection?: RepositoryInspection | null; inspectionLoading?: boolean; inspectionError?: string; connection?: Connection; recheckDefaultBranch?: () => Promise<void>; recheckDisabled?: boolean; checkingDefault?: boolean; inspectionPending?: boolean; refreshInspections?: () => Promise<void>; onSourceRefresh: (value: Connection | undefined) => void; branchPicker: Snippet; children: Snippet<[Snippet, Snippet]> } = $props();
 let onboarding = $state.raw<RepositoryRuntimeRun | null>(null);
 let deploymentEnabled = $state(false), developmentOnly = $state(false);
 let loaded = $state(false);
 let refreshingIntegrations = $state(false);
 let viewedStep = $state<number | null>(null);
 let passedIntegrations = $state('');
 let passedInspection = $state('');
 let passedPreparation = $state('');
 let requestedSigningUpdate = $state('');
 let signingPublicationEnabled = $state(false);
 let signingPublicationBusy = $state(false);
 const pullStates = {
  RUNTIME_SIGNING_PULL_REQUEST_STATE_DRAFT: 'Draft',
  RUNTIME_SIGNING_PULL_REQUEST_STATE_OPEN: 'Open',
  RUNTIME_SIGNING_PULL_REQUEST_STATE_CLOSED: 'Closed',
  RUNTIME_SIGNING_PULL_REQUEST_STATE_MERGED: 'Merged',
 } as const;
 type PullRequest = {url: string; branch: string; state: typeof pullStates[keyof typeof pullStates]};
 type PullStatus = {scope: string; state: 'checking' | 'missing' | 'error'} | {scope: string; state: 'found'; pull: PullRequest};
 let pullStatus = $state.raw<PullStatus>();
 let pullSequence = 0;
 let stepHeading = $state<HTMLHeadingElement>();
 let busy = $state(false), error = $state('');
 let polling = false, mounted = false, historySequence = 0;
 let pending = $state<RuntimeCommand | null>(null);
 let recoveryToast: string | number | undefined;
 let rejected = $state(false);
 let registries = $state.raw<Connection[]>([]), signers = $state.raw<RuntimeSigner[]>([]), operations = $state.raw<RuntimePreparation[]>([]);
 let next = $state(''), registryId = $state(''), signerId = $state(''), platform = $state('linux/arm64');
 let seconds = $state(1800), builds = $state(1), allowBuild = $state(false), allowReplace = $state(false);
 let reviewedSettings = $state('');
 type Integration = 'registry' | 'signer';
 let savedIntegrationRevision = $state<Record<Integration, number>>({registry:0, signer:0});
 const settingsDraft = $derived(JSON.stringify({platform,seconds,builds,allowBuild,allowReplace}));
 protectUnsavedChanges(()=>!pending&&!!reviewedSettings&&settingsDraft!==reviewedSettings);
 const storage = $derived(`ayni.runtime.preparation.pending.${repo.id}`);
 const candidateInspection = $derived(latestInspection);
 const onboardingActive = $derived(!!onboarding && ['resolving','inspecting','admitting'].includes(onboarding.state));
 const onboardingBlocked = $derived(onboarding?.state === 'blocked');
 const workActive = $derived(operations.some(value => activeStates.has(value.state) || value.state === 'publication_uncertain' || !value.cleanupComplete));
 const registry = $derived(registries.find(value => value.id === registryId));
 const signer = $derived(signers.find(value => value.id === signerId));
 const blockingFindings = $derived(candidateInspection?.result?.findings?.filter(finding => !['adapter_discovery_pending', 'runtime_verification_pending'].includes(finding.code)) ?? []);
 const signerTrust = $derived.by(() => {
  const result = candidateInspection?.result;
  if (!signer || candidateInspection?.state !== 'completed' || !result?.lockFingerprint ||
   blockingFindings.some(finding => finding.code === 'trust_invalid' || finding.code === 'inspection_metadata_bounds_exceeded')) return 'unchecked';
  if (!result.trustKeyIds?.includes(signer.keyId)) return 'name_missing';
  if (!result.trustedPublicKeys || !Object.hasOwn(result.trustedPublicKeys, signer.keyId) || !signer.publicKey) return 'unchecked';
  return result.trustedPublicKeys[signer.keyId] === signer.publicKey ? 'matches' : 'key_mismatch';
 });
 const signerTrustMissing = $derived(signerTrust === 'name_missing' || signerTrust === 'key_mismatch');
 const signingKeyStatus = {
  unchecked: 'Public key match not checked',
  name_missing: 'Key name is not trusted',
  key_mismatch: 'Public key does not match',
  matches: 'Trusted public key matches',
 };
 const destination = $derived(registry?.registryDestination ? `${registry.registryDestination}/${repo.fullName.split('/').at(-1)?.toLowerCase()}` : registries.find(value => value.id === onboarding?.settings.registryConnectionId)?.registryDestination ? `${registries.find(value => value.id === onboarding?.settings.registryConnectionId)?.registryDestination}/${repo.fullName.split('/').at(-1)?.toLowerCase()}` : '');
 const onboardingSteps = ['Connect integrations', 'Check ayni.lock', 'Review and start', 'Prepare image', 'Verify and finish'];
 const activeStates = new Set(['pending', 'running', 'stopping', 'cancelling', 'cleanup_pending']);
 const retryStates = new Set(['failed', 'blocked', 'unavailable', 'publication_uncertain']);
 const verificationStages = new Set(['pull_final', 'verify_content', 'verify_compatibility', 'cleanup']);
 const validAllowance = $derived(Number.isInteger(seconds) && seconds >= 1 && seconds <= 86400 && (!allowBuild || Number.isInteger(builds) && builds >= 1 && builds <= 10));
 const sourceReady = $derived(!!connection?.enabled && !!connection.activeCredentialRevision);
 const registryReady = $derived(!!registry?.enabled && !!registry.activeCredentialRevision);
 const signerReady = $derived(!!signer?.enabled);
 const integrationsReady = $derived(sourceReady && registryReady && signerReady);
 const readyIntegrationCount = $derived(Number(sourceReady) + Number(registryReady) + Number(signerReady));
 const reviewedCommit = $derived(
  !inspectionPending && !checkingDefault && latestInspection?.state === 'completed' &&
  latestInspection.commitSha === checkedCommit &&
  latestInspection.connectionRevision === connection?.revision &&
  latestInspection.result &&
  !blockingFindings.length
   ? checkedCommit : undefined
 );
 const validSigningReview = $derived(
  signerReady && !!signer?.publicKey && signerTrustMissing &&
  !inspectionPending && !checkingDefault && candidateInspection?.state === 'completed' &&
  candidateInspection.connectionRevision === connection?.revision && !blockingFindings.length
 );
 const signingUpdateScope = $derived(JSON.stringify([
  branch || repo.defaultBranch, candidateInspection?.commitSha, candidateInspection?.result?.lockFingerprint, connection?.revision, signer?.id, signer?.revision,
 ]));
 const canRequestSigningUpdate = $derived(validSigningReview && candidateInspection?.commitSha === checkedCommit);
 const signingUpdateRequested = $derived(validSigningReview && requestedSigningUpdate === signingUpdateScope &&
  (canRequestSigningUpdate || passedInspection === signingUpdateScope));
 const signingUpdateBase = $derived(branch || repo.defaultBranch);
 const currentPullStatus = $derived(pullStatus?.scope === signingUpdateScope ? pullStatus : undefined);
 const signingPullRequest = $derived(currentPullStatus?.state === 'found' ? currentPullStatus.pull : undefined);
 const signingReview = $derived(signingUpdateRequested && candidateInspection && signer && signingUpdateBase ? {
  repositoryId: repo.id,
  inspectionId: candidateInspection.id,
  expectedConnectionRevision: candidateInspection.connectionRevision,
  signerId: signer.id,
  expectedSignerRevision: signer.revision,
  baseBranch: signingUpdateBase,
 } : undefined);
 const canReview = $derived(signerTrustMissing ? signingUpdateRequested : !!reviewedCommit);
 const retainedRun = $derived(onboarding && (onboarding.settings.branch || '') === branch &&
  onboarding.settings.expectedConnectionRevision === connection?.revision &&
  onboarding.settings.registryConnectionId === registryId && onboarding.settings.signerId === signerId &&
  (!onboarding.settings.expectedRegistryRevision || onboarding.settings.expectedRegistryRevision === registry?.revision) &&
  (!onboarding.settings.expectedSignerRevision || onboarding.settings.expectedSignerRevision === signer?.revision) &&
  (!checkedCommit || (onboarding.commitSha || onboarding.settings.expectedCommitSha) === checkedCommit) ? onboarding : null);
 const overview = $derived(runtimeOverview({loaded,error,rejected,inspectionPending,moreHistory:!!next,pending:!!pending,inspectionLoading,inspectionError,inspectionReason:inspectionReasons[latestInspection?.safeReason || ''],inspection:latestInspection,operations,operationId:retainedRun?.preparationId,registries,signers,sourceReady,sourceRevision:connection?.revision}));
 const operation = $derived(overview.operation);
 const previousOperations = $derived(operations.filter(value => value.id !== overview.operation?.id));
 const matchingOperation = $derived(operation && (retainedRun?.preparationId === operation.id ||
  (!onboarding && !!reviewedCommit && operation.sourceCommit === reviewedCommit)) ? operation : undefined);
 const preparing = $derived(onboardingActive || !!pending || workActive);
 const canViewPreparation = $derived(preparing || !!retainedRun || !!matchingOperation);
 const verificationReached = $derived(!onboardingActive && !!operation && (operation === matchingOperation || workActive) &&
  (operation.state === 'ready' || verificationStages.has(operation.stage)));
 const verified = $derived(!!matchingOperation && matchingOperation.state === 'ready' &&
  matchingOperation.contentVerified && matchingOperation.managedCompatible && matchingOperation.cleanupComplete);
 const image = $derived(runtimeRegistryImage(onboardingActive || onboardingBlocked ? undefined : overview.operation,destination));
 const integrationCheckpoint = $derived(JSON.stringify([connection?.revision, registryId, registry?.revision, signerId, signer?.revision]));
 const inspectionCheckpoint = $derived(signingUpdateRequested ? signingUpdateScope :
  JSON.stringify([branch || repo.defaultBranch, reviewedCommit, connection?.revision, signerId, signer?.revision]));
 const admitted = $derived(!!matchingOperation || !!retainedRun?.preparationId);
 const completed = $derived([
  integrationsReady && (passedIntegrations === integrationCheckpoint || admitted),
  (!!reviewedCommit && canReview && passedInspection === inspectionCheckpoint) || admitted,
  admitted,
  verificationReached && (passedPreparation === operation?.id || !!verified),
  !!verified,
 ]);
 const lastAvailableStep = $derived(completed[3] ? 4 : canViewPreparation ? 3 :
  completed[0] ? (completed[1] || (signingUpdateRequested && passedInspection === inspectionCheckpoint) ? 2 : 1) : 0);
 const currentStep = $derived(Math.min(viewedStep ?? (verified ? 4 : canViewPreparation ? 3 : lastAvailableStep), lastAvailableStep));
 const canStart = $derived(deploymentEnabled && integrationsReady && !!reviewedCommit && !signerTrustMissing &&
  !preparing && !!registry?.enabled && !!registry.activeCredentialRevision && !!signer?.enabled && validAllowance && !busy);

 async function visit(step: number) {
  if (step > lastAvailableStep) return;
  viewedStep = step;
  await tick();
  stepHeading?.focus();
 }
 function advance() {
  const step = currentStep;
  if (step === 0 && integrationsReady) passedIntegrations = integrationCheckpoint;
  if (step === 1 && canReview) passedInspection = inspectionCheckpoint;
  if (step === 3 && verificationReached && operation) passedPreparation = operation.id;
  if (step === 0 || step === 1) saveProgress();
  void visit(step + 1);
 }
 function saveProgress() {
  try {
   localStorage.setItem(runtimeProgressKey(repo.id), JSON.stringify({
    integrations: passedIntegrations, inspection: passedInspection, signingUpdate: requestedSigningUpdate,
   }));
  } catch {
   // The current visit can continue without a saved checkpoint.
  }
 }
 function dismissRecoveryToast() {
  if (recoveryToast !== undefined) toast.dismiss(recoveryToast);
  recoveryToast = undefined;
 }
 function persist(value: RuntimeCommand | null) {
  pending = value;
  rejected = false;
  if (value) sessionStorage.setItem(storage, JSON.stringify(value));
  else {
   sessionStorage.removeItem(storage);
   dismissRecoveryToast();
  }
 }
 async function perform(action: () => Promise<void>) {
  busy = true; error = '';
  try { await action(); } catch (cause) { error = cause instanceof RpcError && cause.status === 409 ? 'The preparation generation or selected configuration changed. Recover the retained command and refresh history before another action.' : cause instanceof Error ? cause.message : 'Preparation could not be loaded. Recover the retained command or refresh history.'; }
  finally {
   busy = false;
   if (mounted && pending) {
    recoveryToast = toast.warning(rejected ? 'Preparation command rejected' : 'Preparation result not confirmed', {
     id: recoveryToast, duration: Infinity,
     description: error || 'Recover the saved receipt before starting another command.',
     action: {label:'Recover', onClick:() => { if (!busy) void perform(recover); }},
    });
   } else if (mounted && error) {
    toast.error(loaded ? 'Preparation needs attention' : 'Could not load runtime onboarding', {description:error});
   }
  }
 }
 async function history(append = false) {
  const sequence = ++historySequence;
  const result = await rpc('ListRuntimePreparations', { repositoryId: repo.id, afterId: append ? next : '', limit: 20 });
  if (!mounted || sequence !== historySequence) return;
  operations = append ? [...operations, ...(result.operations || [])] : result.operations || []; next = result.nextAfterId || '';
 }
 async function configuration(refreshSource = false) {
  const connections: Connection[] = [], identities: RuntimeSigner[] = [];
  let source: Connection | undefined;
  let afterId = '', pages = 0;
  do {
   const page = await rpc('ListConnections', { afterId, limit: 100 });
   for (const connection of (page.connections || []) as Connection[]) {
    if (connection.capability === 'container_registry') connections.push(connection);
    if (connection.id === repo.connectionId) source = connection;
   }
   afterId = page.nextAfterId || '';
  } while (afterId && ++pages < 20);
  const connectionsComplete = !afterId;
  afterId = ''; pages = 0;
  do { const page = await rpc('ListRuntimeSigners', { afterId, limit: 100 }); identities.push(...(page.signers || [])); afterId = page.nextAfterId || ''; } while (afterId && ++pages < 20);
  if (mounted) {
   registries = connections;
   signers = identities;
   if (refreshSource && (source || connectionsComplete)) onSourceRefresh(source);
  }
 }
 function checkedPull(result: {url: string; branch: string; state: keyof typeof pullStates}): PullRequest {
  const url = new URL(result.url);
  const prefix = `/${repo.fullName}/pull/`;
  if (url.origin !== 'https://github.com' || !url.pathname.startsWith(prefix) ||
      !/^[1-9][0-9]*$/.test(url.pathname.slice(prefix.length)) || url.username || url.password ||
      !Object.hasOwn(pullStates, result.state)) {
   throw new Error('The service returned an invalid pull request.');
  }
  return {url: url.href, branch: result.branch, state: pullStates[result.state]};
 }
 async function refreshSigningPullRequest(scope = signingUpdateScope, review = signingReview) {
  if (!review || signingPublicationBusy) return;
  const sequence = ++pullSequence;
  pullStatus = {scope, state:'checking'};
  try {
   const {pullRequest} = await rpc('GetRuntimeSigningPullRequest', {review});
   if (sequence !== pullSequence) return;
   pullStatus = pullRequest ? {scope, state:'found', pull:checkedPull(pullRequest)} : {scope, state:'missing'};
  } catch {
   if (sequence === pullSequence) pullStatus = {scope, state:'error'};
  }
 }
 $effect(() => {
  if (currentStep !== 2 || signingPublicationBusy || !signingPublicationEnabled || !signingReview) return;
  const scope = signingUpdateScope, review = signingReview;
  untrack(() => {
   if (currentPullStatus?.state !== 'found') void refreshSigningPullRequest(scope, review);
  });
 });
 async function openSigningPullRequest() {
  if (!signingPublicationEnabled || signingPublicationBusy || !canRequestSigningUpdate || !signingReview || currentPullStatus?.state !== 'missing') return;
  const scope = signingUpdateScope;
  signingPublicationBusy = true;
  try {
   const {pullRequest} = await rpc('OpenRuntimeSigningPullRequest', signingReview);
   if (mounted && scope === signingUpdateScope) pullStatus = {scope, state:'found', pull:checkedPull(pullRequest)};
  } catch (cause) {
   toast.error('Could not confirm the signing key pull request', {
    description: cause instanceof RpcError && cause.status === 409
     ? 'Check the branch and signing key again before retrying.'
     : 'Refresh the pull request status before trying again.',
   });
   if (mounted && scope === signingUpdateScope) pullStatus = {scope, state:'error'};
  } finally {
   signingPublicationBusy = false;
  }
 }
 async function loadOnboarding() {
  const result = await rpc('GetRepositoryRuntime',{repositoryId:repo.id});
  if (!mounted) return;
  const changed = result.run && (result.run.id !== onboarding?.id || result.run.state !== onboarding?.state);
  onboarding = result.run || null;
  signingPublicationEnabled = result.signingUpdatePublicationEnabled === true;
  deploymentEnabled = result.preparationEnabled === true; developmentOnly = result.developmentOnly === true;
  if (changed && onboarding?.inspectionId) await refreshInspections?.();
 }
 async function refresh() {
  await Promise.all([configuration(), loadOnboarding()]);
  await history();
  if (!loaded) {
   restoreSettings();
   restoreIntegrationSelections();
   reviewedSettings = settingsDraft;
  }
  loaded = true;
 }
 async function refreshIntegrations() {
  refreshingIntegrations = true;
  try {
   await perform(() => configuration(true));
  } finally { refreshingIntegrations = false; }
 }
 function restoreSettings() {
  if (!onboarding?.settings) return;
  const saved=onboarding.settings;
  registryId=saved.registryConnectionId; signerId=saved.signerId; platform=saved.platform;
  seconds=saved.maxSeconds; builds=saved.maxBuilds || 1; allowBuild=saved.allowBuild; allowReplace=saved.allowReplace;
 }
 function integrationStorageKey(kind: Integration) {
  return `ayni.repository.runtime.${kind}.${repo.id}`;
 }
 function restoreIntegrationSelections() {
  const availableRegistries=registries.filter(value=>value.enabled && value.activeCredentialRevision);
  const availableSigners=signers.filter(value=>value.enabled);
  if (!registryId && availableRegistries.length===1) registryId=availableRegistries[0].id;
  if (!signerId && availableSigners.length===1) signerId=availableSigners[0].id;
  try {
   registryId = localStorage.getItem(integrationStorageKey('registry')) ?? registryId;
   signerId = localStorage.getItem(integrationStorageKey('signer')) ?? signerId;
  } catch {
   // The current configuration supplies the defaults when browser storage is unavailable.
  }
 }
 function saveIntegration(kind: Integration, event: Event & {currentTarget: HTMLSelectElement}) {
  const select = event.currentTarget;
  try {
   localStorage.setItem(integrationStorageKey(kind), select.value);
   if (kind === 'registry') registryId = select.value;
   else signerId = select.value;
   savedIntegrationRevision[kind] += 1;
  } catch {
   select.value = kind === 'registry' ? registryId : signerId;
   toast.error('Could not save the integration selection', {description:'Check that this browser allows site storage, then try again.'});
  }
 }

 async function accept(operation: RuntimePreparation) {
  if (operation.repositoryId !== repo.id) throw new Error('The retained preparation belongs to another repository.');
  persist(null);
  const current = await rpc('GetRuntimePreparation', { id: operation.id });
  operations = [current.operation, ...operations.filter(value => value.id !== operation.id)]; await history();
 }
 async function resume() {
  if (!pending) return;
  if (pending.method === 'StartRepositoryRuntime') {
   try { await rpc(pending.method,pending.request); persist(null); await refresh(); }
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
 async function recover() { if (pending?.method === 'StartRepositoryRuntime') { await rpc('GetRepositoryRuntimeReceipt',{idempotencyKey:pending.request.idempotencyKey}); persist(null); await refresh(); return; } if (pending) { const result = await rpc('GetRuntimePreparationReceipt', { idempotencyKey: pending.request.idempotencyKey }); await accept(result.operation); } }
 async function start() {
  if (!connection || !registry || !signer || !reviewedCommit || !canStart) return;
  persist({method:'StartRepositoryRuntime',request:{idempotencyKey:crypto.randomUUID(),repositoryId:repo.id,expectedConnectionRevision:connection.revision,registryConnectionId:registryId,signerId,platform,maxSeconds:seconds,maxBuilds:allowBuild ? builds : 0,allowBuild,allowReplace:allowBuild && allowReplace,branch,expectedCommitSha:reviewedCommit,expectedRegistryRevision:registry.revision,expectedSignerRevision:signer.revision}});
  reviewedSettings = settingsDraft;
  viewedStep = 3;
  await perform(resume);
  await tick();
  stepHeading?.focus();
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
  const progress = readRuntimeProgress(repo.id);
  if (progress) {
   passedIntegrations = progress.integrations;
   passedInspection = progress.inspection;
   requestedSigningUpdate = progress.signingUpdate;
  }
  const saved = sessionStorage.getItem(storage);
  if (saved) {
   try { const value = JSON.parse(saved); if (!['StartRepositoryRuntime', 'StartRuntimePreparation', 'CancelRuntimePreparation', 'RetryRuntimePreparation'].includes(value.method) || typeof value.request?.idempotencyKey !== 'string' || ['StartRepositoryRuntime','StartRuntimePreparation'].includes(value.method) && value.request.repositoryId !== repo.id) throw new Error(); pending = value; viewedStep = 3; void perform(refresh); }
   catch { error = 'The saved preparation command could not be read. Refresh retained history before starting another.'; }
  } else { void perform(refresh); }
  const timer = setInterval(() => void poll(), 2000);
  return () => { mounted = false; clearInterval(timer); dismissRecoveryToast(); };
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
 {#if !loaded}
  <p role="status">Loading runtime onboarding…</p>
  {#if error}<Button onclick={() => perform(refresh)}>Retry</Button>{/if}
 {:else}
  <ol aria-label="Runtime onboarding steps" class="grid gap-2 sm:grid-cols-5">
   {#each onboardingSteps as step, index (step)}
    <li aria-current={index === currentStep ? 'step' : undefined} class="min-w-0">
     <Button variant={completed[index] ? 'default' : 'outline'}
      class={['h-full min-h-10 w-full justify-start whitespace-normal text-left', index === currentStep && 'font-semibold']}
      disabled={index > lastAvailableStep}
      onclick={() => visit(index)}>
      {#if completed[index]}<span class="sr-only">Completed: </span>{/if}
      <span>{index + 1}. {step}</span>
     </Button>
    </li>
   {/each}
  </ol>
  <section class="space-y-4 rounded-xl border bg-card p-5" aria-label="Runtime status">
   <div class="flex items-center justify-between gap-3">
    {#key currentStep}
     <h3 bind:this={stepHeading} tabindex="-1" class="text-lg font-semibold focus:outline-none">{onboardingSteps[currentStep]}</h3>
    {/key}
    {#if currentStep === 0}
     <Button variant="outline" size="icon" aria-label="Refresh integrations" title="Reload available integrations from Configuration"
      aria-busy={refreshingIntegrations} disabled={busy} onclick={refreshIntegrations}>
      <RefreshCw class={{ 'motion-safe:animate-spin': refreshingIntegrations }} aria-hidden="true"/>
     </Button>
    {/if}
   </div>
   {#if currentStep === 0}
    <p role="status" class="text-sm text-muted-foreground">{readyIntegrationCount} of 3 integrations ready</p>
    <InformationTable rows={[
     {label:'Source control connection',value:sourceReady ? connection?.displayName || 'Connected' : 'Unavailable or needs validation'},
    ]}/>
    {#if !sourceReady}<Button variant="link" class="px-0" href="/settings/source-control/github">Manage source connections</Button>{/if}
    <div class="grid gap-4 sm:grid-cols-2">
     <div class="min-w-0 space-y-2">
      <Label for={`runtime-registry-${repo.id}`}>Runtime registry</Label>
      <AutosaveField savedRevision={savedIntegrationRevision.registry}>
       <NativeSelect.Root data-slot="input-group-control" id={`runtime-registry-${repo.id}`} class="w-full" value={registryId}
        onchange={event => saveIntegration('registry', event)} disabled={busy || !!pending}>
        <NativeSelect.Option value="">Choose a registry connection</NativeSelect.Option>
        {#if registryId && !registry}<NativeSelect.Option value={registryId} disabled>Previous registry unavailable</NativeSelect.Option>{/if}
        {#each registries as value (value.id)}
         <NativeSelect.Option value={value.id} disabled={!value.enabled || !value.activeCredentialRevision}>
          {value.displayName}{!value.enabled ? ' (disabled)' : !value.activeCredentialRevision ? ' (validation required)' : ''}
         </NativeSelect.Option>
        {/each}
       </NativeSelect.Root>
      </AutosaveField>
      {#if !registries.some(value => value.enabled && value.activeCredentialRevision)}
       <Button variant="link" class="px-0" href="/settings/container-registries/github">Manage registry connections</Button>
      {/if}
     </div>
     <div class="min-w-0 space-y-2">
      <Label for={`runtime-signer-${repo.id}`}>Runtime signer</Label>
      <AutosaveField savedRevision={savedIntegrationRevision.signer}>
       <NativeSelect.Root data-slot="input-group-control" id={`runtime-signer-${repo.id}`} class="w-full" value={signerId}
        onchange={event => saveIntegration('signer', event)} disabled={busy || !!pending}>
        <NativeSelect.Option value="">Choose a signing key</NativeSelect.Option>
        {#if signerId && !signer}<NativeSelect.Option value={signerId} disabled>Previous signing key unavailable</NativeSelect.Option>{/if}
        {#each signers as value (value.id)}
         <NativeSelect.Option value={value.id} disabled={!value.enabled}>{value.keyId}{!value.enabled ? ' (disabled)' : ''}</NativeSelect.Option>
        {/each}
       </NativeSelect.Root>
      </AutosaveField>
      {#if !signers.some(value => value.enabled)}
       <Button variant="link" class="px-0" href="/settings/runtime-signing">Manage signing keys</Button>
      {/if}
     </div>
    </div>
   {:else if currentStep === 1}
    {@render branchPicker()}
    <p class="text-sm text-muted-foreground">Check the selected branch to inspect the exact commit’s policy, lock, and trust requirements.</p>
    <Button variant="outline" disabled={busy || recheckDisabled || checkingDefault || !!pending || onboardingActive}
     onclick={recheckDefaultBranch}>
     {checkingDefault ? 'Checking ayni.lock…' : 'Check ayni.lock'}
    </Button>
    <div class="space-y-2" aria-live="polite">
     {#if checkingDefault}
      <p class="text-sm">Resolving the selected branch and inspecting its latest commit…</p>
     {:else if candidateInspection}
      <h4 class="text-sm font-medium">Lock check report</h4>
      <InformationTable aligned rows={[
       {label:'ayni.lock check',value:candidateInspection.state === 'accepted' ? 'Pending' :
        candidateInspection.state === 'completed' && candidateInspection.result && !blockingFindings.length ? 'Passed' :
        candidateInspection.state === 'superseded' ? 'Source connection changed' : 'Needs attention'},
       {label:reviewedCommit ? 'Checked commit' : 'Last inspected commit',value:candidateInspection.commitSha,monospace:true},
       {label:'Signing key',value:signer ? `${signer.keyId} · ${signingKeyStatus[signerTrust]}` : 'Choose a signing key in Connect integrations'},
       ...(candidateInspection.result ? [
        {label:'Ayni version',value:candidateInspection.result.ayniVersion || 'Unavailable'},
        {label:'Lock fingerprint',value:candidateInspection.result.lockFingerprint || 'Unavailable',monospace:true},
       ] : []),
      ]}/>
      {#if candidateInspection.state === 'completed' && !reviewedCommit}
       <p class="text-sm text-muted-foreground">Check the selected branch for a current result.</p>
      {/if}
      {#if signerTrustMissing}
       <p class="text-sm">Choose a matching key in Connect integrations, or review the proposed update to the repository trust policy.</p>
       {#if canRequestSigningUpdate}
        <label class="flex items-center gap-2 text-sm">
         <input type="checkbox" class="accent-primary" checked={signingUpdateRequested}
          onchange={event => {
           requestedSigningUpdate = event.currentTarget.checked ? signingUpdateScope : '';
           if (!event.currentTarget.checked) saveProgress();
          }}/>
         Update signing key
        </label>
       {/if}
      {/if}
      {#if candidateInspection.safeReason}<p class="text-sm">{inspectionReasons[candidateInspection.safeReason] || 'The inspection needs attention.'}</p>{/if}
      {#each blockingFindings as finding (finding.code + finding.path)}
       <p class="text-sm">{finding.action}</p>
      {/each}
     {:else}
      <p class="text-sm">No repository check has been saved.</p>
     {/if}
    </div>
   {:else if currentStep === 2 && signingUpdateRequested && signer && candidateInspection}
    <div class="space-y-3">
     <h4 class="font-medium">Signing key update pull request</h4>
     <p class="text-sm">The proposed change pins the selected public key in <code>.ayni.toml</code>, preserves the other trusted keys, and regenerates <code>.ayni.lock</code>.</p>
     <InformationTable aligned rows={[
      {label:'Repository',value:repo.fullName},
      {label:'Target branch',value:signingUpdateBase},
      {label:'Inspected commit',value:candidateInspection.commitSha,monospace:true},
      {label:'Signing key',value:signer.keyId},
     ]}/>
     <pre class="overflow-x-auto rounded-md bg-muted p-2 text-xs">{trustSnippet(signer)}</pre>
     <div class="space-y-3" aria-live="polite">
      {#if signingPullRequest}
       <InformationTable aligned rows={[
        {label:'Pull request status',value:signingPullRequest.state},
        {label:'Pull request branch',value:signingPullRequest.branch,monospace:true},
       ]}/>
      {:else if !signingPublicationEnabled}
       <p class="text-sm">This repository needs a signing update publisher before you can open a pull request here.</p>
      {:else if signingPublicationBusy}
       <p class="text-sm">Opening a draft pull request…</p>
      {:else if currentPullStatus?.state === 'missing'}
       <p class="text-sm">No pull request exists for this signing key update.</p>
      {:else if currentPullStatus?.state === 'error'}
       <p class="text-sm">Pull request status is unavailable. Refresh to try again.</p>
      {:else}
       <p class="text-sm">Checking for an existing pull request…</p>
      {/if}
      <div class="flex flex-wrap gap-2">
       {#if signingPullRequest}
        <Button href={signingPullRequest.url} target="_blank" rel="noopener noreferrer">View PR</Button>
       {:else}
        <Button disabled={!signingPublicationEnabled || signingPublicationBusy || !canRequestSigningUpdate || currentPullStatus?.state !== 'missing'}
         onclick={openSigningPullRequest}>{signingPublicationBusy ? 'Opening PR…' : 'Open PR'}</Button>
       {/if}
       {#if signingPublicationEnabled}
        <Button variant="outline" size="icon" aria-label="Refresh pull request status" title="Refresh pull request status"
         disabled={signingPublicationBusy || currentPullStatus?.state === 'checking'} onclick={() => refreshSigningPullRequest()}>
         <RefreshCw aria-hidden="true"/>
        </Button>
       {/if}
      </div>
     </div>
     {#if signingPullRequest?.state === 'Merged' || !canRequestSigningUpdate}
      <p class="text-sm text-muted-foreground">Check the current branch before continuing runtime preparation.</p>
      <Button variant="outline" disabled={busy || recheckDisabled || checkingDefault || signingPublicationBusy}
       onclick={() => visit(1)}>Go to lock check</Button>
     {:else}
      <p class="text-sm text-muted-foreground">After the pull request is merged, check <code>ayni.lock</code> again to continue runtime preparation.</p>
     {/if}
    </div>
   {:else if currentStep === 2}
    {#if reviewedCommit}<p class="text-sm">Reviewed commit <code class="break-all">{reviewedCommit}</code> on {branch || 'the default branch'}.</p>
    {:else}<p class="text-sm">Recheck the branch before starting a new preparation.</p>{/if}
    {#if !deploymentEnabled}<p class="text-sm">Preparing a runtime image requires a worker profile configured by the deployment administrator.</p>{/if}
    <p class="text-sm text-muted-foreground">Review the registry, signer, and limits for this run. The run continues if you close the browser.</p>
   <InformationTable rows={[
    {label:'Runtime registry',value:registryReady ? registry?.displayName || 'Selected' : 'Choose a registry in Connect integrations'},
    {label:'Runtime signer',value:signerReady ? signer?.keyId || 'Selected' : 'Choose a signing key in Connect integrations'},
   ]}/>
   {#if signer}<details class="rounded-md border p-3"><summary class="cursor-pointer text-sm font-medium">Signing key details</summary><div class="space-y-3 pt-3"><InformationTable rows={[{label:'Key name',value:signer.keyId},{label:'Public key',value:signer.publicKey || 'Unavailable',monospace:true}]}/><Button variant="outline" onclick={() => perform(() => navigator.clipboard.writeText(signer!.publicKey))}>Copy public key</Button><p class="text-xs text-muted-foreground">Use this key name and public key together in the repository’s .ayni.toml trust policy.</p></div></details>{/if}
   {#if signer && signerTrustMissing}
    <div class="space-y-2 rounded-md border p-3">
     <p class="text-sm font-medium">The checked commit does not trust this signing key.</p>
     <p class="text-sm">Return to Connect integrations to choose a key already trusted by this commit.</p>
     <p class="text-sm">To use the selected key, add its public entry to the existing trust policy in <code>.ayni.toml</code> and preserve the other keys. Run <code>ayni env lock</code>, commit both files, then recheck the branch.</p>
     <pre class="overflow-x-auto rounded-md bg-muted p-2 text-xs">{trustSnippet(signer)}</pre>
    </div>
   {/if}
   <div class="flex flex-wrap gap-4"><div class="space-y-2"><Label for={`runtime-platform-${repo.id}`}>Runtime platform</Label><NativeSelect.Root id={`runtime-platform-${repo.id}`} bind:value={platform} disabled={busy || !!pending}><NativeSelect.Option value="linux/arm64">Linux arm64</NativeSelect.Option><NativeSelect.Option value="linux/amd64">Linux amd64</NativeSelect.Option></NativeSelect.Root></div><div class="space-y-2"><Label for={`runtime-seconds-${repo.id}`}>Time limit (seconds)</Label><Input id={`runtime-seconds-${repo.id}`} type="number" min={1} max={86400} bind:value={seconds} disabled={busy || !!pending}/></div></div>
   <label class="flex items-start gap-2 text-sm"><input type="checkbox" bind:checked={allowBuild} onchange={() => { if (!allowBuild) allowReplace = false; }} disabled={busy || !!pending}/>Allow a bounded build if the image is missing</label>
   {#if allowBuild}<div class="space-y-2"><Label for={`runtime-builds-${repo.id}`}>Maximum build attempts</Label><Input id={`runtime-builds-${repo.id}`} type="number" min={1} max={10} bind:value={builds} disabled={busy || !!pending}/><p class="text-xs text-muted-foreground">The qualified deployment resource limits also apply. Interrupted builds consume their retained attempt allowance.</p></div><label class="flex items-start gap-2 text-sm"><input type="checkbox" bind:checked={allowReplace} disabled={busy || !!pending}/>Also allow replacement if an existing image is invalid</label>{/if}
   {:else if !pending}
    <div class="space-y-2" aria-live="polite">
     <p class="font-medium">{onboardingActive ? 'Preparing repository runtime' : onboardingBlocked ? 'Runtime onboarding needs attention' : overview.title}</p>
     <p class="text-sm text-muted-foreground">{onboardingActive ? ({resolving:'Checking the branch head…',inspecting:'Checking the committed Ayni policy, lock and build inputs…',admitting:'Validating build capability, signer trust and preparation settings…'}[onboarding?.state || ''] || 'Preparing…') : onboardingBlocked ? onboardingReasons[onboarding?.safeReason || ''] || 'The runtime setup needs attention. Review the current configuration.' : overview.description}</p>
     {#each overview.issues as issue (issue)}<p class="text-sm">{issue}</p>{/each}
     {#if developmentOnly || operation?.developmentOnly}<p class="rounded-md border p-3 text-sm">Local development build mode. Cluster network isolation is not qualified; this result is for local development testing.</p>{/if}
    </div>
    {#if currentStep === 3 && operation}
     <InformationTable rows={[
      {label:'Build attempts',value:`${operation.buildsStarted || 0} of ${operation.maxBuilds || 0}`},
      {label:'Build permission',value:operation.allowBuild ? 'Build allowed' : 'Reuse an existing image'},
     ]}/>
    {:else if currentStep === 4 && operation}
     <InformationTable rows={[
      {label:'Content verification',value:operation.contentVerified ? 'Verified' : 'Not verified'},
      {label:'Managed compatibility',value:operation.managedCompatible ? 'Verified' : 'Not verified'},
      {label:'Cleanup',value:operation.cleanupComplete ? 'Complete' : 'Not complete'},
     ]}/>
    {/if}
  {#if currentStep === 4 && image}<div class="space-y-2 border-t pt-3"><p class="text-sm font-medium">{image.confirmed ? 'Registry image available' : 'Image destination'}</p><code class="block break-all text-xs">{image.reference || image.destination}</code>{#if !image.confirmed}<p class="text-xs text-muted-foreground">No published image is confirmed by this status yet.</p>{/if}{#if image.reference && verified}
   {@const reference = image.reference}
   <Button variant="outline" onclick={() => perform(() => navigator.clipboard.writeText(reference))}>Copy image reference</Button>
  {/if}<a class="text-sm text-primary underline underline-offset-4" href={image.url} target="_blank" rel="noreferrer">Open registry package</a></div>{/if}
    <div class="flex flex-wrap gap-2">
     <Button variant="outline" disabled={busy} onclick={() => perform(refresh)}>Refresh status</Button>
     {#if operation && retryStates.has(operation.state)}{@const current = operation}<Button disabled={busy || !!pending} onclick={() => perform(() => command('RetryRuntimePreparation',current))}>{current.state === 'publication_uncertain' ? 'Reconcile publication' : 'Retry preparation'}</Button>{/if}
     {#if operation && activeStates.has(operation.state) && operation.state !== 'cancelling'}{@const current = operation}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(() => command('CancelRuntimePreparation',current))}>Cancel preparation</Button>{/if}
    </div>
   {/if}
   {#if pending && currentStep >= 3}
    <div class="flex flex-wrap gap-2" role="group" aria-label="Preparation recovery actions">
     <Button disabled={busy} onclick={() => perform(recover)}>Recover preparation receipt</Button>
     <Button variant="outline" disabled={busy} onclick={() => perform(resume)}>Retry same preparation command</Button>
     {#if rejected}<Button variant="outline" disabled={busy} onclick={() => {persist(null);error='';}}>Dismiss rejected command</Button>{/if}
    </div>
   {/if}
   <div class="flex flex-wrap justify-between gap-2 border-t pt-4">
    {#if currentStep > 0}<Button variant="outline" onclick={() => visit(currentStep - 1)}>Back</Button>{/if}
    {#if currentStep === 0 || currentStep === 1 || currentStep === 3}
     <Button class="ml-auto" disabled={busy || (currentStep === 0 ? !integrationsReady : currentStep === 1 ? !canReview : !verificationReached)} onclick={advance}>Next</Button>
    {:else if currentStep === 2 && !signingUpdateRequested}
     <Button class="ml-auto" disabled={!canStart} onclick={start}>{allowBuild ? 'Build and publish runtime' : 'Start preparation'}</Button>
    {/if}
   </div>
  </section>
 {/if}
</section>
{/snippet}

{#snippet preparationActivity()}
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
