<script lang="ts">
 import { toast } from 'svelte-sonner';
 import { RefreshCw } from '@lucide/svelte';
 import RepositoryCard from '$lib/components/repository-card.svelte';
 import { onMount } from 'svelte';
 import PageHeader from '$lib/components/page-header.svelte';
 import InformationCard from '$lib/components/information-card.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import * as Card from '$lib/components/ui/card/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 import { Label } from '$lib/components/ui/label/index.js';
 import * as NativeSelect from '$lib/components/ui/native-select/index.js';
 import Skeleton from '$lib/components/ui/skeleton/skeleton.svelte';
 import { repositoryOnboardingPath } from '$lib/paths';
 import { connectionAttemptError, connectionAttemptResult, readConnectionAttempts, writeConnectionAttempt, type ConnectionAttempt } from '$lib/repository-onboarding';
 import { rpc, RpcError, type Connection } from '$lib/api';
 import { repositoryReasons, type Repository, type RepositoryOperation, type GithubInstallation, type GithubRepository, type ConnectRequest } from '$lib/repositories';
 let connections=$state.raw<Connection[]>([]);let catalog=$state.raw<Repository[]>([]);
 let discovered=$state.raw<GithubRepository[]>([]);
 let connectionId=$state('');let installationId=$state('');let busy=$state(true);let error=$state('');
 let nextCatalog=$state('');let nextRepositories=$state(0);
 let attempts = $state.raw<Record<string, ConnectionAttempt>>({});
 let connectingId = $state('');
 const pending = $derived(Object.values(attempts).some(attempt => attempt.status === 'pending'));
 let refreshing=$state(false);
 let loadingGrid=$state(true);
 let search=$state('');
 const matchingCatalog = $derived(catalog.filter(repo => repo.fullName.toLowerCase().includes(search.toLowerCase())));
 const visibleAttempts = $derived(Object.values(attempts).filter(({ request, repository }) =>
  !catalog.some(repo => repo.providerRepositoryId === request.providerRepositoryId) &&
  (repository?.fullName ?? request.providerRepositoryId).toLowerCase().includes(search.toLowerCase())
 ));
 const available = $derived(discovered.filter(repo => !attempts[repo.providerRepositoryId] &&
  !catalog.some(item => item.providerRepositoryId === repo.providerRepositoryId) &&
  repo.fullName.toLowerCase().includes(search.toLowerCase())
 ));
 const source = $derived(connections.find(c => c.id === connectionId));
 async function perform(action:()=>Promise<void>){busy=true;error='';try{await action();}catch(e){error=e instanceof RpcError&&e.status===409?'The selection conflicts with current configuration or a retained request. Refresh and review the selection; recover an uncertain request before starting another.':e instanceof RpcError&&e.status===404?'The selected resource is unavailable in this scope. Refresh the catalog and check the project and source connection.':e instanceof Error?e.message:'Repository access could not be checked.';}finally{busy=false;}}
 async function loadGrid(action: () => Promise<void>) {
  loadingGrid = true;
  try { await perform(action); }
  finally { loadingGrid = false; }
 }
 async function load(){const values:Connection[]=[];let afterId='';let pages=0;do{const result=await rpc('ListConnections',{afterId,limit:100});values.push(...(result.connections||[]).filter((c:Connection)=>c.capability==='source_control'));afterId=result.nextAfterId||'';}while(afterId&&++pages<20);const previousRevision=source?.revision;connections=values;if(source?.revision!==previousRevision)changeSource('');await loadCatalog();}
 async function refreshCatalog() {
  refreshing=true;
  try { await loadGrid(load); }
  finally { refreshing=false; }
 }
 async function loadCatalog(append = false) {
  const result = await rpc('ListRepositories', { afterId: append ? nextCatalog : '', limit: 50 });
  const items: Repository[] = result.repositories ?? [];
  catalog = append ? [...new Map([...catalog, ...items].map(item => [item.id, item])).values()] : items;
  nextCatalog = result.nextAfterId ?? '';
  for (const { providerRepositoryId } of items) {
   if (attempts[providerRepositoryId]) attempts = writeConnectionAttempt(providerRepositoryId, null);
  }
 }
 function changeSource(id:string) {
  connectionId=id;
  installationId='';discovered=[];nextRepositories=0;error='';
  if (id) void loadGrid(discoverSource);
 }
 async function discoverSource() {
  if (!source) return;
  const result = await rpc('ListGithubInstallations', { connectionId, expectedConnectionRevision: source.revision, page: 1 });
  if (result.safeReason) throw new Error(repositoryReasons[result.safeReason] || 'Installation discovery failed.');
  const installations: GithubInstallation[] = result.installations ?? [];
  if (installations.length > 1 || result.nextPage) {
   throw new Error('This GitHub App has multiple installations. Use a separate source control connection backed by a GitHub App with one installation.');
  }
  const [installation] = installations;
  if (!installation) {
   toast.warning('No installations found', { description: 'Install this GitHub App on the account that owns your repositories.' });
   return;
  }
  if (installation.suspended) throw new Error('This GitHub App installation is suspended. Restore it on GitHub, then select the connection again.');
  installationId = installation.installationId;
  await findRepositories();
 }
 async function findRepositories(append=false){if(!source||!installationId)return;const result=await rpc('ListGithubRepositories',{connectionId,expectedConnectionRevision:source.revision,installationId,page:append?nextRepositories:1});if(result.safeReason)throw new Error(repositoryReasons[result.safeReason]||'Repository discovery failed.');discovered=[...new Map((append?[...discovered,...(result.repositories||[])]:result.repositories||[]).map((v:GithubRepository)=>[v.providerRepositoryId,v])).values()] as GithubRepository[];nextRepositories=result.nextPage||0;if(!discovered.length)toast.info('No repositories found', { description: 'Check which repositories this installation can access.' });}
 async function connect(repo: GithubRepository) {
  if (!source || pending) return;
  const request: ConnectRequest = {
   idempotencyKey: crypto.randomUUID(), connectionId, expectedConnectionRevision: source.revision,
   installationId, providerRepositoryId: repo.providerRepositoryId,
  };
  const attempt: ConnectionAttempt = { request, repository: repo, status: 'pending' };
  const id = repo.providerRepositoryId;
  attempts = writeConnectionAttempt(id, attempt);
  connectingId = id;
  try {
   const { operation }: { operation: RepositoryOperation } = await rpc('ConnectRepository', request);
   if (operation.state === 'completed' && operation.repository) {
    const connected = operation.repository;
    catalog = [connected, ...catalog.filter(item => item.id !== connected.id)];
   }
   attempts = writeConnectionAttempt(id, connectionAttemptResult(attempt, operation));
  } catch (cause) {
   attempts = writeConnectionAttempt(id, connectionAttemptError(attempt, cause));
  } finally {
   connectingId = '';
  }
 }
 onMount(() => {
  void loadGrid(async () => { attempts = readConnectionAttempts(); await load(); });
 });
</script>
<PageHeader eyebrow="Source code" title="Repositories" />
<div class="space-y-6">
 {#if error}<Feedback kind="error" title="Repository needs attention" message={error}/>{/if}
 <div class="space-y-4">
  <div class="space-y-2">
   <Label for="repository-source">Source control connection</Label>
   <NativeSelect.Root id="repository-source" class="w-full" value={connectionId} onchange={(event)=>changeSource(event.currentTarget.value)} disabled={busy||!!pending}>
    <NativeSelect.Option value="">Choose a source connection</NativeSelect.Option>
    {#each connections as c (c.id)}
     <NativeSelect.Option value={c.id} disabled={!c.enabled||!c.activeCredentialRevision}>{c.displayName}{!c.enabled?' (disabled)':!c.activeCredentialRevision?' (validation required)':''}</NativeSelect.Option>
    {/each}
   </NativeSelect.Root>
   <div class="flex justify-end">
    <Button variant="link" class="h-auto px-0" href="/settings/source-control/github">Add connection</Button>
   </div>
  </div>
  {#if nextRepositories}<Button variant="outline" disabled={busy||!!pending} onclick={()=>perform(()=>findRepositories(true))}>More repositories</Button>{/if}
 </div>
 <div class="flex items-center justify-between">
  <h2 id="available-repositories" class="text-lg font-semibold">Available repositories</h2>
  <Button variant="outline" size="icon" disabled={busy} aria-label="Refresh repositories" title="Refresh repositories" aria-busy={refreshing} onclick={refreshCatalog}>
   <RefreshCw class={refreshing ? 'size-4 motion-safe:animate-spin' : 'size-4'} aria-hidden="true" />
  </Button>
 </div>
 {#if !loadingGrid&&!catalog.length}<p class="text-sm text-muted-foreground">No repositories connected yet.</p>{/if}
 <div class="flex flex-wrap items-center justify-between gap-4"><p class="text-sm text-muted-foreground">{catalog.length} connected {catalog.length===1?'repository':'repositories'}</p><Input class="max-w-xs" bind:value={search} aria-label="Search repositories" placeholder="Search repositories…"/></div>
 <div class="grid gap-5 sm:grid-cols-2 xl:grid-cols-3" role="region" aria-labelledby="available-repositories" aria-busy={loadingGrid}>
 {#if loadingGrid}
  {#each [1, 2, 3] as slot (slot)}
   <Card.Root aria-hidden="true">
    <Card.Header><Skeleton class="my-1 h-5 w-2/3" /></Card.Header>
    <Card.Content class="space-y-4">
     <div class="min-h-16 space-y-2">
      <Skeleton class="h-4 w-1/2" />
      <Skeleton class="h-4 w-2/3" />
     </div>
     <Skeleton class="h-8 w-36 max-w-full" />
    </Card.Content>
   </Card.Root>
  {/each}
 {:else}
 {#each matchingCatalog as repo (repo.id)}<RepositoryCard {repo}/>{/each}
 {#each visibleAttempts as attempt (attempt.request.providerRepositoryId)}
  {@const id = attempt.request.providerRepositoryId}
  {@const repo = attempt.repository ?? discovered.find(item => item.providerRepositoryId === id)}
  {@const href = repositoryOnboardingPath(id)}
  <InformationCard title={repo?.fullName ?? `Repository ${id}`} headingLevel={2}
   status={id === connectingId ? 'connecting' : attempt.status === 'failed' ? 'connection_failed' : 'pending'}
   navigationHref={id === connectingId ? undefined : href}>
   {#if repo}<p class="text-sm leading-relaxed text-muted-foreground">{repo.private ? 'Private repository' : 'Public repository'}<br />Default branch: {repo.defaultBranch}</p>{/if}
   {#if id === connectingId}
    <p role="status" class="text-sm text-muted-foreground">Checking source access…</p>
   {:else}
    <Button variant="outline" href={href}>Repository settings</Button>
    {#if attempt.status === 'failed' && source && repo && discovered.some(item => item.providerRepositoryId === id)}
     <Button disabled={busy || pending} onclick={() => perform(() => connect(repo))}>Connect repository</Button>
    {/if}
   {/if}
  </InformationCard>
 {/each}
 {#each available as repo (repo.providerRepositoryId)}
  <InformationCard title={repo.fullName} headingLevel={2}>
   <p class="min-h-16 text-sm leading-relaxed text-muted-foreground">{repo.private?'Private repository':'Public repository'}<br/>Default branch: {repo.defaultBranch}</p>
   <Button disabled={busy||!!pending} onclick={()=>perform(()=>connect(repo))}>Connect repository</Button>
  </InformationCard>
 {/each}
 {/if}
 </div>
 {#if !loadingGrid&&search&&!matchingCatalog.length&&!available.length&&!visibleAttempts.length}<p class="text-sm text-muted-foreground">No matching repositories.</p>{/if}
 {#if nextCatalog}<Button variant="outline" disabled={busy} onclick={()=>perform(()=>loadCatalog(true))}>More catalog repositories</Button>{/if}
</div>
