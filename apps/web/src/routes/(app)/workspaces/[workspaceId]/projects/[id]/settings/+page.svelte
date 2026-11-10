<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
  import InformationCard from '$lib/components/information-card.svelte';
  import { workspaceSettingsPath } from '$lib/paths';
  import PageHeader from '$lib/components/page-header.svelte';
  import RenameHierarchyDialog from '$lib/components/rename-hierarchy-dialog.svelte';
  import ProjectAgentSettings from '$lib/components/project-agent-settings.svelte';
  let { data } = $props();
</script>

<PageHeader eyebrow="Project settings" title={data.project?.displayName ?? 'Project settings'} />
<PageContent>
  {#if data.workspace && data.project}
    {#key data.project.id}
      <RenameHierarchyDialog inline kind="project" id={data.project.id} workspaceId={data.workspace.id} name={data.project.displayName} resourceVersion={data.project.resourceVersion} />
      <InformationCard title="Project goal">
        <p class="whitespace-pre-wrap text-sm">{data.project.goal}</p>
        <p class="text-sm text-muted-foreground">Repositories are shared through {data.workspace.displayName}. <a class="underline" href={workspaceSettingsPath(data.workspace.id)}>Manage workspace repositories</a></p>
      </InformationCard>
      <ProjectAgentSettings workspaceId={data.workspace.id} projectId={data.project.id} />
    {/key}
  {:else}
    <p>Choose a project from its workspace.</p>
  {/if}
</PageContent>
