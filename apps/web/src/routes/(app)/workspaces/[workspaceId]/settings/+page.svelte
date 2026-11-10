<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
 import PageHeader from '$lib/components/page-header.svelte';
 import WorkspaceRepositories from '$lib/components/workspace-repositories.svelte';
 import RenameHierarchyDialog from '$lib/components/rename-hierarchy-dialog.svelte';
 let {data}=$props();
</script>
<PageHeader
 eyebrow={data.workspace ? "Workspace settings" : undefined}
 title={data.workspace?.displayName ?? "Workspace settings"}
/>
<PageContent>
 {#if data.workspace}
  {#key data.workspace.id}
   <RenameHierarchyDialog inline kind="workspace" id={data.workspace.id} name={data.workspace.displayName} resourceVersion={data.workspace.resourceVersion} />
   <WorkspaceRepositories workspaceId={data.workspace.id} />
  {/key}
 {:else}
  <p>Create or choose a workspace first.</p>
 {/if}
</PageContent>
