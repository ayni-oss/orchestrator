<script lang="ts">
  import type { Snippet } from "svelte";
  import { MediaQuery } from "svelte/reactivity";
  import type { Workspace, WorkspaceData } from "$lib/workspace-types";
  import { afterNavigate, goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Layers, Plus, ChevronDown, EllipsisVertical, Settings2, PanelLeftClose, PanelLeftOpen, Menu, X, GitBranch, Package, BrainCircuit, PanelsTopLeft } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import SidebarItem from "$lib/components/sidebar-item.svelte";
  import CreateWorkspaceDialog from "$lib/components/create-workspace-dialog.svelte";
  import { workspaceActions } from "$lib/hierarchy-actions";
  import * as Dropdown from "$lib/components/ui/dropdown-menu/index.js";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import AppBreadcrumb from "$lib/components/app-breadcrumb.svelte";
  let { data, title = "Projects", breadcrumbs, children }: {
    data: WorkspaceData; title?: string; breadcrumbs: { href: string; label: string }[]; children: Snippet;
  } = $props();
  let sidebarOpen = $state(false);
  const compactViewport = new MediaQuery("(width <= 800px)");
  let collapsePreference = $state<boolean>();
  const sidebarCollapsed = $derived(collapsePreference ?? compactViewport.current);
  let createWorkspaceOpen = $state(false);
  let configurationOpen = $state(page.url.pathname.startsWith("/settings"));
  function closeConfigurationOnOutsideClick(event: MouseEvent) {
    if (!configurationOpen) return;
    const clickedConfiguration = event.composedPath().some(target =>
      target instanceof Element &&
      target.matches('#configuration-navigation, [aria-controls="configuration-navigation"]')
    );
    if (!clickedConfiguration) configurationOpen = false;
  }
  function closeConfiguration() {
    configurationOpen = false;
  }
  afterNavigate(({ type }) => {
    sidebarOpen = false;
    if (type !== 'enter') closeConfiguration();
  });
</script>
{#snippet workspaceOption(workspace: Workspace)}
  {@const selected = workspace.id === data.workspace?.id}
  <div role="presentation" class="flex items-center gap-1">
    <Dropdown.Item
      class="min-w-0 flex-1"
      aria-current={selected ? "true" : undefined}
      onSelect={() => { void goto(`/?workspace=${workspace.id}`); }}
    >
      <span class="truncate">{workspace.displayName}</span>
    </Dropdown.Item>
    <Dropdown.Sub>
      <Dropdown.SubTrigger
        class="w-8 justify-center"
        aria-label={`Actions for ${workspace.displayName}`}
        title={`Actions for ${workspace.displayName}`}
        showChevron={false}
      >
        <EllipsisVertical aria-hidden="true" />
      </Dropdown.SubTrigger>
      <Dropdown.SubContent>
        {#each workspaceActions(workspace.id) as action (action.label)}
          <Dropdown.Item onSelect={action.onSelect}>{action.label}</Dropdown.Item>
        {/each}
      </Dropdown.SubContent>
    </Dropdown.Sub>
  </div>
{/snippet}
<svelte:document onclickcapture={closeConfigurationOnOutsideClick} />
<a class="skip" href="#main-content">Skip to content</a>
<div class:configuration-open={configurationOpen} class:sidebar-collapsed={sidebarCollapsed} class="workspace-shell">
  <aside id="workspace-navigation" class:mobile-open={sidebarOpen} aria-label="Workspace navigation">
    <a class="brand" href="/" aria-label="Ayni Orchestrator home" title="Ayni Orchestrator home"><span class="brand-mark" aria-hidden="true"></span><span class="sidebar-label">ayni<small>ORCHESTRATOR</small></span></a>
    <p class="section-label">Workspaces</p>
    <div class="workspace-actions flex min-w-0 items-center gap-2">
      <Button variant="outline" size="icon-lg" aria-label="Create workspace" title="Create workspace" onclick={() => { sidebarOpen = false; createWorkspaceOpen = true; }}><Plus /></Button>
      <Dropdown.Root>
        <Dropdown.Trigger>
          {#snippet child({ props })}
            <Button {...props} variant="outline" size="lg" class="workspace-picker min-w-0 flex-1 justify-between" aria-label="Select workspace" title={data.workspace?.displayName ?? "Select workspace"}>
              <PanelsTopLeft class="rail-icon" />
              <span class="sidebar-label truncate">{data.workspace?.displayName ?? 'Select workspace'}</span>
              <ChevronDown class="sidebar-label" />
            </Button>
          {/snippet}
        </Dropdown.Trigger>
        <Dropdown.Content align="start" class="min-w-52">
          <Dropdown.Label>Workspaces</Dropdown.Label>
          {#if data.workspace && !data.workspaces.some(workspace => workspace.id === data.workspace?.id)}
            {@render workspaceOption(data.workspace)}
          {/if}
          {#each data.workspaces as workspace (workspace.id)}
            {@render workspaceOption(workspace)}
          {/each}
          <Dropdown.Separator />
          <Dropdown.Item onSelect={() => { void goto('/'); }}>All workspaces</Dropdown.Item>
        </Dropdown.Content>
      </Dropdown.Root>
    </div>
    <nav aria-label="Projects" class="mt-7 min-h-0 overflow-y-auto">
      <SidebarItem label="Projects" href={data.workspace ? `/?workspace=${data.workspace.id}` : '/'} disabled={!data.workspace} active={title==='Projects'}><Layers /><span class="sidebar-label">Projects</span></SidebarItem>
      {#if data.workspace && data.projects.length}
        <ul class="sidebar-label ml-5 mt-1 space-y-1 border-l border-border pl-2">
          {#each data.projects as project (project.id)}
            <li><SidebarItem href={`/projects/${project.id}/repositories?workspace=${data.workspace.id}`} active={page.params.id===project.id}><span class="min-w-0 truncate">{project.displayName}</span></SidebarItem></li>
          {/each}
          {#if data.nextProject}<li><SidebarItem href={`/?workspace=${data.workspace.id}&afterProject=${data.nextProject}`}>More projects</SidebarItem></li>{/if}
        </ul>
      {/if}
    </nav>
    <div class="sidebar-bottom">
      <p class="section-label">Settings</p>
      <SidebarItem label="Repositories" href="/repositories" active={title==='Repositories'}><GitBranch /><span class="sidebar-label">Repositories</span></SidebarItem>
      <SidebarItem label="Configuration" active={configurationOpen} expanded={configurationOpen} controls="configuration-navigation" onclick={() => { configurationOpen = !configurationOpen; sidebarOpen = false; }}><Settings2 /><span class="sidebar-label">Configuration</span></SidebarItem>
    </div>
  </aside>
  {#if sidebarOpen}<Button variant="ghost" class="mobile-backdrop" onclick={() => sidebarOpen = false} aria-label="Close workspace navigation"></Button>{/if}
  {#if configurationOpen}
    <nav id="configuration-navigation" class="configuration-navigation" aria-label="Configuration navigation">
      <div class="configuration-heading"><h2>Configuration</h2><Button variant="ghost" onclick={closeConfiguration} aria-label="Close configuration navigation"><X size={17} /></Button></div>
      <p class="configuration-group">Integrations</p>
      <SidebarItem href="/settings/source-control" onclick={closeConfiguration} active={page.url.pathname.startsWith('/settings/source-control')}><GitBranch />Source control</SidebarItem>
      <SidebarItem href="/settings/container-registries" onclick={closeConfiguration} active={page.url.pathname.startsWith('/settings/container-registries')}><Package />Container registries</SidebarItem>
      <SidebarItem href="/settings/runtime-signing" onclick={closeConfiguration} active={page.url.pathname.startsWith('/settings/runtime-signing')}><Package />Runtime signing</SidebarItem>
      <SidebarItem href="/settings/llm-providers" onclick={closeConfiguration} active={page.url.pathname.startsWith('/settings/llm-providers')}><BrainCircuit />LLM providers</SidebarItem>
    </nav>
  {/if}
  <CreateWorkspaceDialog bind:open={createWorkspaceOpen} />
  <div class="main-shell">
    <header class="topbar">
      <Button variant="ghost" class="mobile-toggle" onclick={() => sidebarOpen = !sidebarOpen} aria-label="Toggle workspace navigation" aria-expanded={sidebarOpen} aria-controls="workspace-navigation">{#if sidebarOpen}<PanelLeftClose size={20} />{:else}<Menu size={20} />{/if}</Button>
      <Button
        variant="ghost"
        size="icon-lg"
        class="desktop-sidebar-toggle"
        onclick={() => collapsePreference = !sidebarCollapsed}
        aria-label={sidebarCollapsed ? "Expand workspace navigation" : "Collapse workspace navigation"}
        aria-expanded={!sidebarCollapsed}
        aria-controls="workspace-navigation"
      >
        {#if sidebarCollapsed}<PanelLeftOpen class="size-5" />{:else}<PanelLeftClose class="size-5" />{/if}
      </Button>
      <AppBreadcrumb items={breadcrumbs} />
      <ThemeToggle />
    </header>
    <main id="main-content">{@render children()}</main>
  </div>
</div>

<style>
  .workspace-shell{--sidebar-width:240px;display:grid;grid-template-columns:var(--sidebar-width) minmax(0,1fr);min-height:100vh}.workspace-shell.configuration-open{grid-template-columns:var(--sidebar-width) 260px minmax(0,1fr)}aside{position:sticky;top:0;height:100vh;background:var(--sidebar);border-right:1px solid var(--border);padding:30px 18px 16px;display:flex;flex-direction:column;gap:0}.brand{color:var(--primary);display:flex;align-items:center;gap:12px;margin:0 12px 36px;text-decoration:none;font-size:28px;font-weight:650;letter-spacing:-1px}.brand-mark{width:33px;height:33px;background:var(--primary);mask:url("/ayni-orchestrator.svg") center/contain no-repeat;flex-shrink:0}.brand small{display:block;font:9px var(--font-mono);letter-spacing:2px;color:var(--muted-foreground);margin-top:2px}.section-label{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin:30px 12px 14px}.sidebar-bottom{margin-top:auto;padding-top:30px}.configuration-navigation{position:sticky;top:0;height:100vh;overflow-y:auto;padding:28px 14px;background:var(--card);border-right:1px solid var(--border)}.configuration-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;padding:0 10px 16px}.configuration-heading h2{font-size:17px;font-weight:600}.configuration-heading :global(button){display:grid;place-items:center;width:28px;height:28px;color:var(--muted-foreground)}.configuration-group{margin:15px 10px 7px;font:10px var(--font-mono);text-transform:uppercase;letter-spacing:1px;color:var(--muted-foreground)}.main-shell{min-width:0}.topbar{height:76px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between;padding:0 40px;gap:15px}main{padding:40px;max-width:1560px;margin:0 auto}.skip{position:fixed;top:-100px;left:20px;z-index:100;background:var(--primary);color:var(--primary-foreground);padding:12px}.skip:focus{top:10px}:global(.mobile-toggle),:global(.mobile-backdrop){display:none}@media(max-width:1200px){main{padding:32px}.topbar{padding:0 32px}}@media(max-width:540px){.workspace-shell,.workspace-shell.configuration-open{grid-template-columns:1fr}aside{display:none}aside.mobile-open{display:flex;position:fixed;left:0;top:0;width:240px;z-index:50}:global(.mobile-backdrop){display:block;position:fixed;inset:0;width:100vw;height:100dvh;padding:0;border:0;border-radius:0;background:#0008;z-index:40}.configuration-navigation{position:fixed;left:0;top:64px;bottom:0;height:auto;width:min(300px,100vw);z-index:45;box-shadow:6px 0 20px #0002}:global(.mobile-toggle){display:block}.topbar{height:64px;padding:0 22px}main{padding:30px 22px}}
  .workspace-actions :global(.rail-icon) { display: none; }

  @media (width > 540px) {
    .workspace-shell.sidebar-collapsed { --sidebar-width: 64px; }
    .sidebar-collapsed aside { padding-inline: 8px; }
    .sidebar-collapsed .brand {
      justify-content: center;
      margin-inline: 0;
    }
    .sidebar-collapsed aside .section-label,
    .sidebar-collapsed aside .sidebar-label,
    .sidebar-collapsed .workspace-actions :global(svg.sidebar-label) {
      display: none;
    }
    .sidebar-collapsed .workspace-actions { flex-direction: column; }
    .sidebar-collapsed .workspace-actions :global(.workspace-picker) {
      flex: none;
      width: 36px;
      justify-content: center;
      padding: 0;
    }
    .sidebar-collapsed .workspace-actions :global(.rail-icon) { display: block; }
    .sidebar-collapsed aside :global([data-slot="button"]) { justify-content: center; }
  }

  @media (540px < width <= 800px) {
    .workspace-shell.configuration-open {
      grid-template-columns: var(--sidebar-width) minmax(0, 1fr);
    }
    .configuration-navigation {
      position: fixed;
      top: 76px;
      bottom: 0;
      left: var(--sidebar-width);
      width: 260px;
      height: auto;
      z-index: 45;
      box-shadow: 6px 0 20px #0002;
    }
  }

  @media (width <= 540px) {
    .topbar :global(.desktop-sidebar-toggle) { display: none; }
  }
</style>
