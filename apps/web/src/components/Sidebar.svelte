<script>
  let { view, client, oncreate, mobileOpen = $bindable(false) } = $props();
</script>
<aside class:mobile-open={mobileOpen} aria-label="Workspace navigation">
  <button class="icon-button mobile-close" aria-label="Close navigation" onclick={() => mobileOpen = false}>×</button>
  <a class="brand" href="/workspace/"><img src="/workspace/ayni-orchestrator.svg" alt="" width="34" height="34"><span>ayni<small>ORCHESTRATOR</small></span></a>
  <div class="sidebar-heading"><label for="workspace-select">Workspace</label><button class="icon-button" onclick={() => oncreate('workspace')} aria-label="Create workspace">+</button></div>
  <select id="workspace-select" disabled={view.busy || view.sending} value={view.scope.workspaceId || ''} onchange={event => client.select({ workspaceId: event.currentTarget.value })}>
    <option value="">Choose a workspace</option>
    {#each view.workspaces as workspace (workspace.id)}<option value={workspace.id}>{workspace.displayName}</option>{/each}
  </select>
  {#if view.pages.workspaces}<button class="text-button" onclick={() => client.more('workspaces')} disabled={view.busy}>Load more workspaces</button>{/if}
  <div class="sidebar-heading"><h2>Projects</h2><button class="icon-button" onclick={() => oncreate('project')} disabled={!view.scope.workspaceId || view.busy || view.sending || !!view.pending} aria-label="Create project">+</button></div>
  <nav class="project-list" aria-label="Projects">
    {#each view.projects as project (project.id)}
      <button class:chosen={project.id === view.scope.projectId} aria-current={project.id === view.scope.projectId ? 'page' : undefined} disabled={view.busy || view.sending} onclick={() => { mobileOpen = false; client.select({ workspaceId: view.scope.workspaceId, projectId: project.id }); }}><span class="project-initial">{project.displayName.slice(0, 1)}</span><span>{project.displayName}</span></button>
    {:else}<p class="muted sidebar-empty">{view.scope.workspaceId ? 'Create a project to bring your work together.' : 'Choose or create a workspace.'}</p>{/each}
  </nav>
  {#if view.pages.projects}<button class="text-button" onclick={() => client.more('projects')} disabled={view.busy}>Load more projects</button>{/if}
  <footer><span class="local-dot"></span> Local workspace<p>Isolated agents. Retained work.</p><button class="text-button" disabled={view.busy || view.sending} onclick={() => client.select(view.scope)}>Refresh resources</button></footer>
</aside>
