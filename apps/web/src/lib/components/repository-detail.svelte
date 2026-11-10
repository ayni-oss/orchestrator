<script lang="ts">
 import { onMount } from 'svelte';
 import { page } from '$app/state';
 import { Tabs } from 'bits-ui';
 import { Badge } from '$lib/components/ui/badge/index.js';
 import { Button } from '$lib/components/ui/button/index.js';
 import PageHeader from './page-header.svelte';
 import InformationTable from './information-table.svelte';
 import RepositoryInspection from './repository-inspection.svelte';
 import Feedback from './feedback.svelte';
 import { rpc, type Connection } from '$lib/api';
 import type { Repository } from '$lib/repositories';
 let { repositoryId }: { repositoryId: string } = $props();
 let repo = $state.raw<Repository | null>(null);
 let connection = $state.raw<Connection | undefined>();
 let busy = $state(true), error = $state('');
 function initialTab() {
  const requested = page.url.searchParams.get('tab');
  return requested === 'source' || requested === 'activity' ? requested : 'runtime';
 }
 let tab = $state(initialTab());
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
<PageHeader eyebrow={repo ? "Repository" : undefined} title={repo?.fullName ?? "Repository"}/>
<div class="max-w-5xl space-y-5">
 {#if busy}<p role="status" class="text-sm text-muted-foreground">Loading repository…</p>{:else if error}<Feedback kind="error" title="Repository unavailable" message={error}/><Button variant="outline" onclick={load}>Try again</Button>
 {:else if repo}
 <div class="flex flex-wrap gap-2">
  <Badge variant="outline">{repo.private ? 'Private repository' : 'Public repository'}</Badge>
  <Badge variant="outline" class="h-auto max-w-full whitespace-normal [overflow-wrap:anywhere]">Default branch: {repo.defaultBranch}</Badge>
 </div>
 <Tabs.Root bind:value={tab}>
  <Tabs.List aria-label={`${repo.fullName} onboarding`} class="mb-6 flex gap-3 border-b">
   <Tabs.Trigger value="runtime" class={triggerClass}>ayni runtime</Tabs.Trigger>
   <Tabs.Trigger value="source" class={triggerClass}>Source code</Tabs.Trigger>
   <Tabs.Trigger value="activity" class={triggerClass}>Activity</Tabs.Trigger>
  </Tabs.List>
  <Tabs.Content value="source" class="space-y-4">
   <h2 class="font-semibold">Source integration details</h2>
   <InformationTable rows={[
    {label:'Source control connection',value:connection?.displayName || 'Unavailable'},
    {label:'Connection status',value:!connection ? 'Unavailable' : !connection.enabled ? 'Disabled' : !connection.activeCredentialRevision ? 'Validation required' : 'Configured'},
    {label:'Repository ID',value:repo.providerRepositoryId},
    {label:'Installation ID',value:repo.installationId},
    {label:'Connected',value:new Date(Number(repo.connectedAtUnixMs)).toLocaleString()},
   ]}/>
  </Tabs.Content>
  <RepositoryInspection {repo} bind:connection/>
 </Tabs.Root>
 {/if}
</div>
