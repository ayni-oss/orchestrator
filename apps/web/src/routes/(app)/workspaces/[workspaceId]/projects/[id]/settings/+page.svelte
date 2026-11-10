<script lang="ts">
  import InformationCard from '$lib/components/information-card.svelte';
  import PageContent from '$lib/components/page-content.svelte';
  import ProjectGoalSettings from '$lib/components/project-goal-settings.svelte';
  import PageHeader from '$lib/components/page-header.svelte';
  import RenameHierarchyDialog from '$lib/components/rename-hierarchy-dialog.svelte';
  import ProjectAgentSettings from '$lib/components/project-agent-settings.svelte';
  let { data } = $props();
</script>

<PageHeader eyebrow="Project settings" title={data.project?.displayName ?? 'Project settings'} />
<PageContent>
  {#if data.workspace && data.project}
    {#key data.project.id}
      <InformationCard title="Project configuration">
        <RenameHierarchyDialog inline kind="project" id={data.project.id} workspaceId={data.workspace.id} name={data.project.displayName} resourceVersion={data.project.resourceVersion} />
        <ProjectGoalSettings project={data.project} />
      </InformationCard>
      <ProjectAgentSettings workspaceId={data.workspace.id} projectId={data.project.id} />
    {/key}
  {:else}
    <p>Choose a project from its workspace.</p>
  {/if}
</PageContent>
