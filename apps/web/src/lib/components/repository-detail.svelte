<script lang="ts">
 import { onMount } from 'svelte';
 import { goto } from '$app/navigation';
 import { page } from '$app/state';
 import { Tabs } from 'bits-ui';
 import { Eye, EyeOff, GitBranch, Info } from '@lucide/svelte';
 import { Button, buttonVariants } from '$lib/components/ui/button/index.js';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import AyniMark from './ayni-mark.svelte';
 import PageHeader from './page-header.svelte';
 import InformationTable from './information-table.svelte';
 import RepositoryInspection from './repository-inspection.svelte';
 import Feedback from './feedback.svelte';
 import StatusBadge from './status-badge.svelte';
 import { rpc, type Connection } from '$lib/api';
 import type { StatusTone } from '$lib/presentation';
 import type { Repository } from '$lib/repositories';
 let { repositoryId }: { repositoryId: string } = $props();
 let repo = $state.raw<Repository | null>(null);
 let connection = $state.raw<Connection | undefined>();
 let busy = $state(true), error = $state('');
 function initialTab() {
  return page.url.searchParams.get('tab') === 'runtime' ? 'runtime' : 'status';
 }
 let tab = $state(initialTab());
 function selectTab(value: string) {
  tab = value;
  const url = new URL((page.shallow?.url ?? page.url).href);
  url.searchParams.set('tab', value);
  void goto(url, {state:page.state, shallow:true, replace:true});
 }
 let sourceDetailsOpen = $state(page.url.searchParams.get('details') === 'source' || page.url.searchParams.get('tab') === 'source');
 function clearSourceDetailsLink(open: boolean) {
  if (open) return;
  const currentUrl = page.shallow?.url ?? page.url;
  const url = new URL(currentUrl.href);
  if (url.searchParams.get('details') === 'source') url.searchParams.delete('details');
  if (url.searchParams.get('tab') === 'source') url.searchParams.delete('tab');
  if (url.href !== currentUrl.href) void goto(url, {state:page.state, shallow:true, replace:true});
 }
 const headerReady = $derived(!!repo && !busy && !error);
 const connectionStatus: {label: string; tone: StatusTone} = $derived(
  !connection ? {label:'Unavailable',tone:'neutral'} :
  !connection.enabled ? {label:'Disabled',tone:'neutral'} :
  !connection.activeCredentialRevision ? {label:'Validation required',tone:'warning'} :
  {label:'Configured',tone:'success'}
 );
 const metadataClass = buttonVariants({variant:'secondary',class:'pointer-events-none h-auto min-h-8 max-w-full whitespace-normal py-1'});
 const triggerClass = 'border-b-2 border-transparent px-3 py-3 text-sm font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[state=active]:border-primary data-[state=active]:text-foreground';
 async function load() {
  busy = true; error = '';
  try {
   let afterId = '';
   do {
    const result = await rpc('ListRepositories', { afterId, limit: 100 });
    repo = result.repositories?.find((value: Repository) => value.id === repositoryId) || null;
    afterId = result.nextAfterId || '';
   } while (!repo && afterId);
   if (!repo) throw new Error('This repository is unavailable. Return to repositories and refresh the catalog.');
   afterId = '';
   do {
    const result = await rpc('ListConnections', { afterId, limit: 100 });
    connection = result.connections?.find((value: Connection) => value.id === repo?.connectionId);
    afterId = result.nextAfterId || '';
   } while (!connection && afterId);
  } catch (cause) { error = cause instanceof Error ? cause.message : 'Repository could not be loaded.'; }
  finally { busy = false; }
 }
 onMount(() => { void load(); });
</script>

{#snippet sourceConnectionStatus()}
 <span class="inline-flex"><span class="sr-only">Source connection status</span><StatusBadge status={connectionStatus.label} tone={connectionStatus.tone} /></span>
{/snippet}

{#snippet sourceDetailsAction()}
 {#if repo}
  <Dialog.Root bind:open={sourceDetailsOpen} onOpenChange={clearSourceDetailsLink}>
   <Dialog.Trigger>
    {#snippet child({props})}
     <Button {...props} variant="ghost" size="icon" aria-label="View source integration details" title="View source integration details"><Info aria-hidden="true" /></Button>
    {/snippet}
   </Dialog.Trigger>
   <Dialog.Content class="max-h-[85dvh] overflow-y-auto sm:max-w-lg" preventScroll>
    <Dialog.Header class="pr-8">
     <Dialog.Title>Source integration details</Dialog.Title>
     <Dialog.Description>Source control connection and repository identifiers.</Dialog.Description>
    </Dialog.Header>
    <InformationTable aligned rows={[
     {label:'Source control connection',value:connection?.displayName || 'Unavailable'},
     {label:'Repository ID',value:repo.providerRepositoryId,monospace:true},
     {label:'Installation ID',value:repo.installationId,monospace:true},
     {label:'Connected',value:new Date(Number(repo.connectedAtUnixMs)).toLocaleString()},
    ]}/>
   </Dialog.Content>
  </Dialog.Root>
 {/if}
{/snippet}

<PageHeader eyebrow={repo ? 'Repository' : undefined} title={repo?.fullName ?? 'Repository'}
 titleBadge={headerReady ? sourceConnectionStatus : undefined} actions={headerReady ? sourceDetailsAction : undefined}/>
<div class="max-w-5xl space-y-5">
 {#if busy}<p role="status" class="text-sm text-muted-foreground">Loading repository…</p>{:else if error}<Feedback kind="error" title="Repository unavailable" message={error}/><Button variant="outline" onclick={load}>Try again</Button>
 {:else if repo}
 <div class="flex flex-wrap justify-end gap-2" role="group" aria-label="Repository metadata">
  <span class={metadataClass}>
   {#if repo.private}<EyeOff aria-hidden="true" />Private{:else}<Eye aria-hidden="true" />Public{/if}
  </span>
  <span class={metadataClass}>
   <GitBranch aria-hidden="true" />
   <span class="sr-only">Default branch</span>
   <span class="min-w-0 [overflow-wrap:anywhere]">{repo.defaultBranch}</span>
  </span>
 </div>
 <Tabs.Root value={tab} onValueChange={selectTab}>
  <Tabs.List aria-label={`${repo.fullName} onboarding`} class="mb-6 flex gap-3 border-b">
   <Tabs.Trigger value="status" class={triggerClass}>Status</Tabs.Trigger>
   <Tabs.Trigger value="runtime" class={`inline-flex items-center gap-2 ${triggerClass}`}><AyniMark class="size-4" />ayni</Tabs.Trigger>
  </Tabs.List>
  <RepositoryInspection {repo} bind:connection/>
 </Tabs.Root>
 {/if}
</div>
