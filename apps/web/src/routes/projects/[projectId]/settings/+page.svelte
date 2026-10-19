<script lang="ts">
  import type { PageProps } from './$types';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { ArrowLeft } from '@lucide/svelte';
  import WorkspaceShell from '$lib/components/workspace-shell.svelte';
  import PageHeader from '$lib/components/page-header.svelte';
  import ProjectSettings from '$lib/components/project-settings.svelte';
  import ApplicationNotifications from '$lib/components/application-notifications.svelte';
  import Feedback from '$lib/components/feedback.svelte';
  import * as Tabs from '$lib/components/ui/tabs/index.js';
  let { data }: PageProps = $props();
  const tab = $derived(page.url.searchParams.get('tab') === 'repositories' ? 'repositories' : 'settings');
  function showTab(value: string) {
    const url = new URL(page.url);
    if (value === 'repositories') url.searchParams.set('tab', 'repositories');
    else url.searchParams.delete('tab');
    void goto(url, { keepFocus: true, noScroll: true });
  }
</script>
<svelte:head><title>{data.project?.displayName ?? 'Project'} settings · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<WorkspaceShell {data} title="Project settings">
  {#if data.project}
    <div class="settings-page">
      <a class="back" href={`/projects/${encodeURIComponent(data.project.id)}`}><ArrowLeft size={16}/> Back to {data.project.displayName}</a>
      <PageHeader flush eyebrow="Project / Configuration" title={`${data.project.displayName} settings`} description="Change this project’s goal, repositories, and agent configuration." />
      <Tabs.Root value={tab} onValueChange={showTab} class="gap-6">
        <Tabs.List aria-label="Project configuration"><Tabs.Trigger value="settings">Settings</Tabs.Trigger><Tabs.Trigger value="repositories">Repositories</Tabs.Trigger></Tabs.List>
        <Tabs.Content value="settings">{#key data.project.id}<ProjectSettings {data} section="settings" />{/key}</Tabs.Content>
        <Tabs.Content value="repositories">{#key data.project.id}<ProjectSettings {data} section="repositories" />{/key}</Tabs.Content>
      </Tabs.Root>
    </div>
  {:else}<Feedback kind="error" title="Project unavailable" message={data.error?.message ?? 'This project could not be loaded.'}/>{/if}
</WorkspaceShell>
<style>
  .settings-page{max-width:1024px;margin:0 auto;padding-bottom:60px}.back{display:inline-flex;align-items:center;gap:8px;font-size:12px;color:var(--muted-foreground);margin-bottom:32px}.back:hover{color:var(--foreground)}
</style>
