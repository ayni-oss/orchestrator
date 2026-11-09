<script lang="ts">
  import { onMount } from 'svelte';
  import { toast } from 'svelte-sonner';
  import { rpc, RpcError } from '$lib/api';
  import type { Repository } from '$lib/repositories';
  import InformationCard from '$lib/components/information-card.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import * as NativeSelect from '$lib/components/ui/native-select/index.js';

  let { workspaceId }: { workspaceId: string } = $props();
  type MembershipChange = {
    idempotencyKey: string;
    workspaceId: string;
    repositoryId: string;
    available: boolean;
  };
  let catalog = $state.raw<Repository[]>([]);
  let members = $state.raw<Repository[]>([]);
  let nextCatalog = $state('');
  let repositoryId = $state('');
  let pending = $state<MembershipChange | null>(null);
  let busy = $state(true);
  let error = $state('');
  const storageKey = $derived(`ayni.workspace.${workspaceId}.repository.pending`);
  const memberIds = $derived(new Set(members.map(repository => repository.id)));
  const choices = $derived(catalog.filter(repository => !memberIds.has(repository.id)));

  async function perform(action: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await action();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Workspace repositories could not be loaded.';
    } finally {
      busy = false;
    }
  }

  async function loadCatalog(append = false) {
    const result = await rpc('ListRepositories', { afterId: append ? nextCatalog : '', limit: 50 });
    const repositories = result.repositories ?? [];
    catalog = append ? [...catalog, ...repositories] : repositories;
    nextCatalog = result.nextAfterId || '';
  }

  async function loadMembers() {
    const repositories: Repository[] = [];
    let afterId = '';
    do {
      const result = await rpc('ListWorkspaceRepositories', { workspaceId, afterId, limit: 100 });
      repositories.push(...(result.repositories ?? []));
      afterId = result.nextAfterId || '';
    } while (afterId);
    members = repositories;
  }

  function clearPending() {
    pending = null;
    sessionStorage.removeItem(storageKey);
  }

  async function retry() {
    const change = pending;
    if (!change) return;
    try {
      await rpc('SetWorkspaceRepository', change);
    } catch (cause) {
      if (cause instanceof RpcError && [400, 404, 409].includes(cause.status)) clearPending();
      throw cause;
    }
    clearPending();
    repositoryId = '';
    toast.success(change.available ? 'Repository added to workspace' : 'Repository removed from workspace');
    await loadMembers();
  }

  function changeMembership(id: string, available: boolean) {
    if (!id || busy || pending) return;
    pending = { idempotencyKey: crypto.randomUUID(), workspaceId, repositoryId: id, available };
    sessionStorage.setItem(storageKey, JSON.stringify(pending));
    void perform(retry);
  }

  onMount(() => {
    void perform(async () => {
      const saved = sessionStorage.getItem(storageKey);
      if (saved) {
        const change: MembershipChange = JSON.parse(saved);
        if (change.workspaceId === workspaceId) pending = change;
      }
      await Promise.all([loadCatalog(), loadMembers()]);
    });
  });
</script>

<InformationCard title="Workspace repositories">
  <p class="text-sm text-muted-foreground">Add connected repositories so projects in this workspace can use them.</p>
  {#if error}<Feedback kind="error" title="Workspace repositories need attention" message={error} />{/if}
  {#if pending}
    <div class="space-y-2">
      <p class="text-sm">Retry the saved change to recover its result.</p>
      <Button variant="outline" disabled={busy} onclick={() => perform(retry)}>Retry repository change</Button>
    </div>
  {/if}
  <div class="space-y-2">
    <Label for="workspace-repository">Connected repository</Label>
    <NativeSelect.Root id="workspace-repository" class="w-full" bind:value={repositoryId} disabled={busy || !!pending}>
      <NativeSelect.Option value="">Choose a repository</NativeSelect.Option>
      {#each choices as repository (repository.id)}
        <NativeSelect.Option value={repository.id}>{repository.fullName}</NativeSelect.Option>
      {/each}
    </NativeSelect.Root>
  </div>
  <div class="flex flex-wrap gap-2">
    <Button disabled={busy || !!pending || !choices.some(repository => repository.id === repositoryId)} onclick={() => changeMembership(repositoryId, true)}>Add repository</Button>
    <Button variant="link" href="/repositories">Open repository catalog</Button>
    {#if nextCatalog}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(() => loadCatalog(true))}>More catalog repositories</Button>{/if}
    <Button variant="outline" disabled={busy || !!pending} onclick={() => perform(async () => { await Promise.all([loadCatalog(), loadMembers()]); })}>Refresh workspace repositories</Button>
  </div>
  {#if members.length}
    <ul class="divide-y divide-border">
      {#each members as repository (repository.id)}
        <li class="flex items-center justify-between gap-3 py-2">
          <span class="min-w-0 break-words">{repository.fullName}</span>
          <Button variant="ghost" disabled={busy || !!pending} aria-label={`Remove ${repository.fullName}`} onclick={() => changeMembership(repository.id, false)}>Remove</Button>
        </li>
      {/each}
    </ul>
  {:else if !busy}
    <p class="text-sm text-muted-foreground">No repositories are available to this workspace yet.</p>
  {/if}
</InformationCard>
