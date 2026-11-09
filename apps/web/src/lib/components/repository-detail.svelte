<script lang="ts">
 import { onMount } from 'svelte';
 import { page } from '$app/state';
 import { Tabs } from 'bits-ui';
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
 let tab = $state(page.url.searchParams.get('tab') === 'runtime' ? 'runtime' : 'source');
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
<PageHeader eyebrow={repo ? "Repository" : undefined} title={repo?.fullName ?? "Repository"} description={repo ? `${repo.private ? 'Private repository' : 'Public repository'} · Default branch: ${repo.defaultBranch}` : undefined}/>
<div class="max-w-5xl space-y-5">
 {#if busy}<p role="status" class="text-sm text-muted-foreground">Loading repository…</p>{:else if error}<Feedback kind="error" title="Repository unavailable" message={error}/><Button variant="outline" onclick={load}>Try again</Button>
 {:else if repo}
 <Tabs.Root bind:value={tab}>
  <Tabs.List aria-label={`${repo.fullName} onboarding`} class="mb-6 flex gap-3 border-b">
   <Tabs.Trigger value="source" class={triggerClass}>Source code</Tabs.Trigger>
   <Tabs.Trigger value="runtime" class={triggerClass}>Ayni runtime</Tabs.Trigger>
  </Tabs.List>
  <Tabs.Content value="source" class="space-y-4">
   <div class="space-y-1"><h3 class="font-semibold">Source code connected</h3><p class="text-sm text-muted-foreground">This repository is in the shared catalog and can be selected by projects. Set up Ayni runtime to check the latest default branch and build, publish and verify its image.</p></div>
    <InformationTable rows={[{label:'GitHub connection',value:connection?.displayName || 'Connection unavailable'},{label:'Connection status',value:!connection ? 'Unavailable' : !connection.enabled ? 'Disabled' : !connection.activeCredentialRevision ? 'Validation required' : 'Configured'}]}/>
    {#if !connection?.enabled || !connection?.activeCredentialRevision}<p class="text-sm">Enable and validate the source connection before inspecting commits.</p>{/if}
    <Button onclick={() => {tab='runtime';}}>Set up runtime</Button>
    <a class="text-sm text-primary underline underline-offset-4" href="/settings/source-control/github">Manage source connection</a>
    <details class="rounded-md border p-3"><summary class="cursor-pointer text-sm font-medium">Source integration details</summary><div class="pt-3"><InformationTable rows={[{label:'GitHub repository ID',value:repo.providerRepositoryId},{label:'GitHub installation ID',value:repo.installationId},{label:'Connected',value:new Date(Number(repo.connectedAtUnixMs)).toLocaleString()}]}/><p class="mt-3 text-xs text-muted-foreground">GitHub IDs identify this repository and the App installation that grants access. Source access was checked when the repository was connected.</p></div></details>
  </Tabs.Content>
  <Tabs.Content value="runtime" class="space-y-4"><RepositoryInspection {repo} {connection}/></Tabs.Content>
 </Tabs.Root>
 {/if}
</div>
