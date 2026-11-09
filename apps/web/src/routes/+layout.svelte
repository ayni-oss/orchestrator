<script lang="ts">
 import { onMount } from 'svelte';
 import { page } from '$app/state';
 import WorkspaceShell from '$lib/components/workspace-shell.svelte';
 import '../app.css';
 let { data, children } = $props();
 const title=$derived(page.url.pathname.startsWith('/repositories')?'Repositories':page.url.pathname.startsWith('/projects/')?'Project repositories':page.url.pathname.startsWith('/settings/runtime-signing')?'Runtime signing':page.url.pathname.startsWith('/settings/container-registries')?'Container registries':page.url.pathname.startsWith('/settings/llm-providers')?'LLM providers':page.url.pathname.startsWith('/settings')?'Source control':page.url.pathname.startsWith('/workspace/settings')?'Workspace settings':data.workspace?'Projects':'Workspaces');
 const breadcrumbs=$derived.by(() => {
  const workspaces = { href: '/', label: 'Workspaces' };
  const workspace = data.workspace && {
   href: `/?workspace=${data.workspace.id}`,
   label: data.workspace.displayName,
  };
  const current = { href: page.url.pathname + page.url.search, label: title };
  switch (page.route.id) {
   case '/': return workspace ? [workspaces, { ...current, label: workspace.label }] : [current];
   case '/workspace/settings':
   case '/projects/[id]/repositories':
    return workspace ? [workspaces, workspace, current] : [workspaces, current];
   case '/repositories/[id]':
    return [{ href: '/repositories', label: 'Repositories' }, { ...current, label: 'Repository' }];
   case '/settings/source-control/github':
    return [{ href: '/settings/source-control', label: 'Source control' }, { ...current, label: 'GitHub' }];
   case '/settings/container-registries/github':
    return [{ href: '/settings/container-registries', label: 'Container registries' }, { ...current, label: 'GitHub' }];
   case '/settings/llm-providers/openai':
    return [{ href: '/settings/llm-providers', label: 'LLM providers' }, { ...current, label: 'OpenAI' }];
   default: return [current];
  }
 });
 onMount(()=>{document.documentElement.dataset.hydrated='true';});
</script>
<svelte:head><title>{title} · Ayni Orchestrator</title></svelte:head>
<WorkspaceShell {data} {title} {breadcrumbs}>{@render children()}</WorkspaceShell>
