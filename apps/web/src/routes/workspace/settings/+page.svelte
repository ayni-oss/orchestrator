<script lang="ts">
 import PageHeader from '$lib/components/page-header.svelte';
 import WorkspaceAgentSettings from '$lib/components/workspace-agent-settings.svelte';
 let {data}=$props();
</script>
<div class="mx-auto max-w-5xl space-y-8"><PageHeader flush title="Workspace settings" description="Choose the connection, model and execution budgets for this workspace."/>
{#if data.workspace}{#key data.workspace.id}<WorkspaceAgentSettings workspaceId={data.workspace.id}/>{/key}{:else}<p>Create or choose a workspace first.</p>{/if}
</div>
