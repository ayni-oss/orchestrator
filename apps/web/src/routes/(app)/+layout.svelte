<script lang="ts">
 import { Toaster } from 'svelte-sonner';
 import { onMount } from 'svelte';
 import { workspacePath } from '$lib/paths';
 import { page } from '$app/state';
 import WorkspaceShell from '$lib/components/workspace-shell.svelte';

 let { data, children } = $props();
 const title=$derived(page.url.pathname.startsWith('/repositories')?'Repositories':!!page.params.id && !!page.params.workspaceId?'Project settings':page.url.pathname.startsWith('/settings/runtime-signing')?'Runtime signing':page.url.pathname.startsWith('/settings/container-registries')?'Container registries':page.url.pathname.startsWith('/settings/llm-providers')?'LLM providers':page.url.pathname.startsWith('/settings')?'Source control':page.url.pathname.endsWith('/settings') && !!page.params.workspaceId?'Workspace settings':data.workspace?'Projects':'Workspaces');
 const breadcrumbs=$derived.by(() => {
  const workspaces = { href: '/', label: 'Workspaces' };
  const workspace = data.workspace && {
   href: workspacePath(data.workspace.id),
   label: data.workspace.displayName,
  };
  const current = { href: page.url.pathname + page.url.search, label: title };
  switch (page.route.id) {
   case '/(app)':
   case '/(app)/workspaces/[workspaceId]': return workspace ? [workspaces, { ...current, label: workspace.label }] : [current];
   case '/(app)/workspaces/[workspaceId]/settings':
   case '/(app)/workspaces/[workspaceId]/projects/[id]/settings':
    return workspace ? [workspaces, workspace, current] : [workspaces, current];
   case '/(app)/repositories/[id]':
    return [{ href: '/repositories', label: 'Repositories' }, { ...current, label: 'Repository' }];
   case '/(app)/settings/source-control/github':
    return [{ href: '/settings/source-control', label: 'Source control' }, { ...current, label: 'GitHub' }];
   case '/(app)/settings/container-registries/github':
    return [{ href: '/settings/container-registries', label: 'Container registries' }, { ...current, label: 'GitHub' }];
   case '/(app)/settings/llm-providers/openai':
    return [{ href: '/settings/llm-providers', label: 'LLM providers' }, { ...current, label: 'OpenAI' }];
   default: return [current];
  }
 });
 onMount(()=>{document.documentElement.dataset.hydrated='true';});
</script>
<svelte:head><title>{title} · Ayni Orchestrator</title></svelte:head>
<WorkspaceShell {data} {title} {breadcrumbs}>{@render children()}</WorkspaceShell>
<Toaster closeButton pauseWhenPageIsHidden style="--normal-bg: var(--popover); --normal-text: var(--popover-foreground); --normal-border: var(--border); --border-radius: var(--radius);" />
