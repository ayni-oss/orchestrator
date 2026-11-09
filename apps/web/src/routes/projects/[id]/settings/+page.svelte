<script lang="ts">
  import PageHeader from '$lib/components/page-header.svelte';
  import RenameHierarchyDialog from '$lib/components/rename-hierarchy-dialog.svelte';
  import ProjectAgentSettings from '$lib/components/project-agent-settings.svelte';
  let { data } = $props();
</script>

<PageHeader eyebrow="Project settings" title={data.project?.displayName ?? 'Project settings'} />
<div class="mx-auto max-w-5xl space-y-8">
  {#if data.workspace && data.project}
    {#key data.project.id}
      <RenameHierarchyDialog inline kind="project" id={data.project.id} workspaceId={data.workspace.id} name={data.project.displayName} resourceVersion={data.project.resourceVersion} />
      <ProjectAgentSettings workspaceId={data.workspace.id} projectId={data.project.id} />
    {/key}
  {:else}
    <p>Choose a project from its workspace.</p>
  {/if}
</div>
