<script lang="ts">
  import { tick } from 'svelte';
  import { goto } from '$app/navigation';
  import { Plus } from '@lucide/svelte';
  import RepositoryPicker from '$lib/components/repository-picker.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import * as Dialog from '$lib/components/ui/dialog/index.js';

  let { open = $bindable(false), showTrigger = false }: { open?: boolean; showTrigger?: boolean } = $props();
  let repositories = $state<Array<{ id: string; displayName: string }>>([]);
  let members = $state<string[]>([]);
  let name = $state('');
  let loaded = $state(false);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state('');
  let idempotencyKey = crypto.randomUUID();

  $effect(() => { if (open && !loaded) { loaded = true; void loadRepositories(); } });

  async function loadRepositories() {
    loading = true;
    error = '';
    try {
      const response = await fetch('/workspaces/new');
      const result = await response.json() as { repositories?: Array<{ id: string; displayName: string }>; error?: string };
      if (!response.ok) throw new Error(result.error ?? 'Repositories could not be loaded.');
      repositories = result.repositories ?? [];
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Repositories could not be loaded.';
    } finally { loading = false; }
  }

  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = '';
    try {
      const response = await fetch('/workspaces/new', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ idempotencyKey, displayName: name, repositoryConnectionIds: members }),
      });
      const result = await response.json() as { id?: string; error?: string };
      if (!response.ok || !result.id) throw new Error(result.error ?? 'Workspace could not be created.');
      open = false;
      await tick();
      await goto(`/workspaces/${encodeURIComponent(result.id)}/settings`);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Workspace could not be created.';
    } finally { saving = false; }
  }
</script>

{#if showTrigger}<Button onclick={() => open = true}><Plus size={16} />Create workspace</Button>{/if}
<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-xl">
    <Dialog.Header><Dialog.Title>Create workspace</Dialog.Title><Dialog.Description>Choose the repositories projects in this workspace can use.</Dialog.Description></Dialog.Header>
    <form onsubmit={create} class="space-y-5">
      <div class="space-y-2"><Label for="new-workspace-name">Workspace name</Label><Input id="new-workspace-name" bind:value={name} required maxlength={256} autocomplete="off" /></div>
      <div class="space-y-2"><Label for="new-workspace-repositories">Repositories</Label><RepositoryPicker id="new-workspace-repositories" {repositories} value={members} onchange={value => members = value} /></div>
      {#if loading}<p class="text-sm text-muted-foreground">Loading repositories…</p>{:else if !repositories.length && !error}<p class="text-sm text-muted-foreground">Connect a repository before creating a workspace.</p><Button href="/repositories" variant="outline">Connect repositories</Button>{/if}
      {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{#if !repositories.length}<Button type="button" variant="outline" onclick={loadRepositories}>Retry loading</Button>{/if}{/if}
      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false}>Cancel</Button><Button type="submit" disabled={saving || loading || !members.length}>{saving ? 'Creating…' : 'Create workspace'}</Button></Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
