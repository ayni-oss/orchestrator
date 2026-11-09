<script lang="ts">
  import { onMount } from 'svelte';
  import { toast } from 'svelte-sonner';
  import { rpc, RpcError } from '$lib/api';
  import type { Repository } from '$lib/repositories';
  import InformationCard from '$lib/components/information-card.svelte';
  import Skeleton from '$lib/components/ui/skeleton/skeleton.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  import { Button } from '$lib/components/ui/button/index.js';

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
  let pending = $state<MembershipChange | null>(null);
  let busy = $state(true);
  let loading = $state(true);
  let membershipLoaded = $state(false);
  let error = $state('');
  const storageKey = $derived(`ayni.workspace.${workspaceId}.repository.pending`);
  const memberIds = $derived(new Set(members.map(repository => repository.id)));
  const repositories = $derived([
    ...catalog,
    ...members.filter(repository => !catalog.some(entry => entry.id === repository.id)),
  ]);

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

  async function loadRepositories() {
    loading = true;
    try {
      await Promise.all([loadCatalog(), loadMembers()]);
    } finally {
      loading = false;
    }
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
    membershipLoaded = true;
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
    const repository = repositories.find(entry => entry.id === change.repositoryId);
    if (change.available && repository && !memberIds.has(repository.id)) {
      members = [...members, repository];
    }
    if (!change.available) members = members.filter(entry => entry.id !== change.repositoryId);
    clearPending();
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
      await loadRepositories();
    });
  });
</script>

<InformationCard title="Workspace repositories">
  {#if error}<Feedback kind="error" title="Workspace repositories need attention" message={error} />{/if}
  {#if pending && !busy}
    <div class="space-y-2">
      <p class="text-sm">Retry the saved change to recover its result.</p>
      <Button variant="outline" disabled={busy} onclick={() => perform(retry)}>Retry repository change</Button>
    </div>
  {/if}
  <fieldset class="space-y-2" disabled={busy || !!pending} aria-busy={busy}>
    <legend class="text-sm font-medium">Select repositories</legend>
    {#if loading}
      <div class="grid grid-cols-2 gap-2 rounded-md border border-input p-2 dark:bg-input/30" aria-hidden="true">
        <Skeleton class="h-5 w-3/4" />
        <Skeleton class="h-5 w-2/3" />
      </div>
    {:else if membershipLoaded && repositories.length}
      <div class="grid grid-cols-2 gap-2 rounded-md border border-input dark:bg-input/30">
        {#each repositories as repository (repository.id)}
          {@const selected = memberIds.has(repository.id)}
          <label class="flex min-w-0 cursor-pointer items-center gap-2 rounded-md p-2 text-sm hover:bg-muted/50">
            <input
              type="checkbox"
              value={repository.id}
              class="size-4 shrink-0 accent-primary"
              checked={selected}
              onclick={(event) => {
                event.preventDefault();
                changeMembership(repository.id, !selected);
              }}
            />
            <span class="min-w-0 [overflow-wrap:anywhere]">{repository.fullName}</span>
          </label>
        {/each}
      </div>
    {:else if membershipLoaded && !busy}
      <p class="text-sm text-muted-foreground">Connect a repository in the catalog to add it here.</p>
    {/if}
  </fieldset>
  <div class="flex flex-wrap gap-2">
    <Button variant="link" href="/repositories">Open repository catalog</Button>
    {#if nextCatalog}<Button variant="outline" disabled={busy || !!pending} onclick={() => perform(() => loadCatalog(true))}>More catalog repositories</Button>{/if}
    <Button variant="outline" disabled={busy || !!pending} onclick={() => perform(loadRepositories)}>Refresh workspace repositories</Button>
  </div>
</InformationCard>
