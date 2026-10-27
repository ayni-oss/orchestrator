<script lang="ts">
 import { onMount } from 'svelte';
 import { page } from '$app/state';
 import WorkspaceShell from '$lib/components/workspace-shell.svelte';
 import '../app.css';
 let { data, children } = $props();
 const title=$derived(page.url.pathname.startsWith('/settings/container-registries')?'Container registries':page.url.pathname.startsWith('/settings/llm-providers')?'LLM providers':page.url.pathname.startsWith('/settings')?'Source control':page.url.pathname.startsWith('/workspace/settings')?'Workspace settings':'Projects');
 onMount(()=>{document.documentElement.dataset.hydrated='true';});
</script>
<svelte:head><title>{title} · Ayni Orchestrator</title></svelte:head>
<WorkspaceShell {data} {title}>{@render children()}</WorkspaceShell>
