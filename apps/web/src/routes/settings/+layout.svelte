<script lang="ts">
  import type { LayoutProps } from './$types';
  import { page } from '$app/state';
  import WorkspaceShell from '$lib/components/workspace-shell.svelte';

  let { data, children }: LayoutProps = $props();
  const title = $derived(page.url.pathname.startsWith('/settings/source-control') ? 'Source control'
    : page.url.pathname.startsWith('/settings/container-registries') ? 'Container registries'
    : page.url.pathname.startsWith('/settings/llm-providers') ? 'LLM providers' : 'Ayni');
</script>

<WorkspaceShell {data} {title}>
  {@render children()}
</WorkspaceShell>
