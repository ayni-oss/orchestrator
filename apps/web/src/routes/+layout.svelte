<script lang="ts">
 import { onMount } from 'svelte';
 import { page } from '$app/state';
 import WorkspaceShell from '$lib/components/workspace-shell.svelte';
 import '../app.css';
 let { data, children } = $props();
 const title=$derived(page.url.pathname.startsWith('/repositories')?'Repositories':page.url.pathname.startsWith('/projects/')?'Project repositories':page.url.pathname.startsWith('/settings/runtime-signing')?'Runtime signing':page.url.pathname.startsWith('/settings/container-registries')?'Container registries':page.url.pathname.startsWith('/settings/llm-providers')?'LLM providers':page.url.pathname.startsWith('/settings')?'Source control':page.url.pathname.startsWith('/workspace/settings')?'Workspace settings':data.workspace?'Projects':'Workspaces');
 const back=$derived.by(() => {
  const projects = data.workspace
   ? { href: `/?workspace=${data.workspace.id}`, label: 'Back to projects' }
   : { href: '/', label: 'Back to workspaces' };
  switch (page.route.id) {
   case '/': return data.workspace ? { href: '/', label: 'Back to workspaces' } : undefined;
   case '/workspace/settings':
   case '/projects/[id]/repositories': return projects;
   case '/repositories/[id]': return { href: '/repositories', label: 'Back to repositories' };
   case '/settings/source-control/github': return { href: '/settings/source-control', label: 'Back to source control' };
   case '/settings/container-registries/github': return { href: '/settings/container-registries', label: 'Back to container registries' };
   case '/settings/llm-providers/openai': return { href: '/settings/llm-providers', label: 'Back to LLM providers' };
   default: return undefined;
  }
 });
 onMount(()=>{document.documentElement.dataset.hydrated='true';});
</script>
<svelte:head><title>{title} · Ayni Orchestrator</title></svelte:head>
<WorkspaceShell {data} {title} {back}>{@render children()}</WorkspaceShell>
