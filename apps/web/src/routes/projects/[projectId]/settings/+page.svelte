<script lang="ts">
  import type { PageProps } from './$types';
  import { ArrowLeft } from '@lucide/svelte';
  import WorkspaceShell from '$lib/components/workspace-shell.svelte';
  import type { PageData as LegacyProjectPageData } from '../../../_old/projects/[projectId]/$types';
  import ProjectSettings from '$lib/components/project-settings.svelte';
  import ApplicationNotifications from '$lib/components/application-notifications.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  let { data }: PageProps = $props();
</script>
<svelte:head><title>{data.project?.displayName ?? 'Project'} settings · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<WorkspaceShell {data} title="Project settings">
  {#if data.project}
    <div class="settings-page">
      <a class="back" href={`/projects/${encodeURIComponent(data.project.id)}`}><ArrowLeft size={16}/> Back to {data.project.displayName}</a>
      <p class="eyebrow">Project / Settings</p>
      <h1>{data.project.displayName} settings</h1>
      <p class="description">Change this project's goal, repositories, and agent configuration.</p>
      {#key data.project.id}<ProjectSettings data={data as unknown as LegacyProjectPageData}/>{/key}
    </div>
  {:else}<Feedback kind="error" title="Project unavailable" message={data.error?.message ?? 'This project could not be loaded.'}/>{/if}
</WorkspaceShell>
<style>
  .settings-page{max-width:820px;margin:0 auto;padding-bottom:60px}.back{display:inline-flex;align-items:center;gap:8px;font-size:12px;color:var(--muted-foreground);margin-bottom:32px}.back:hover{color:var(--foreground)}.eyebrow{font:10px var(--font-mono);letter-spacing:1px;text-transform:uppercase;color:var(--muted-foreground);margin-bottom:10px}h1{font-size:28px;font-weight:600;letter-spacing:-.5px}.description{font-size:13px;color:var(--muted-foreground);margin:9px 0 32px}
</style>
