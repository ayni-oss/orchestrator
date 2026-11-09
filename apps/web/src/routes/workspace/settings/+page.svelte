<script lang="ts">
 import PageHeader from '$lib/components/page-header.svelte';
 import WorkspaceAgentSettings from '$lib/components/workspace-agent-settings.svelte';
 import RenameHierarchyDialog from '$lib/components/rename-hierarchy-dialog.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 let {data}=$props();
 let renameDialog=$state<RenameHierarchyDialog>();
</script>
{#snippet workspaceSettingsActions()}
 <Button variant="outline" onclick={() => renameDialog?.show()}>Rename workspace</Button>
{/snippet}
<PageHeader
 eyebrow={data.workspace ? "Workspace settings" : undefined}
 title={data.workspace?.displayName ?? "Workspace settings"}
 actions={data.workspace ? workspaceSettingsActions : undefined}
/>
<div class="mx-auto max-w-5xl space-y-8">
 {#if data.workspace}
  {#key data.workspace.id}
   <RenameHierarchyDialog bind:this={renameDialog} kind="workspace" id={data.workspace.id} name={data.workspace.displayName} resourceVersion={data.workspace.resourceVersion} />
   <WorkspaceAgentSettings workspaceId={data.workspace.id}/>
  {/key}
 {:else}
  <p>Create or choose a workspace first.</p>
 {/if}
</div>
