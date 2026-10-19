<script lang="ts">
  import type { Snippet } from "svelte";
  import type { WorkspaceDto, ProjectDto } from "@ayni-orchestrator/bff/server";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { LayoutDashboard, Layers, Activity, ChevronDown, Settings2, Plus, Check, PanelLeftClose, Menu, EllipsisVertical, X, GitBranch, Container, BrainCircuit } from "@lucide/svelte";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import AyniIcon from "$lib/components/ayni-icon.svelte";
  import * as Dropdown from "$lib/components/ui/dropdown-menu/index.js";
  let { data, title = "Projects", children }: { data: { workspace?: WorkspaceDto | undefined; workspaces: WorkspaceDto[]; projects: ProjectDto[] }; title?: string; children: Snippet } = $props();
  let sidebarOpen = $state(false);
  let configurationOpen = $state(page.url.pathname.startsWith("/settings") || (page.url.pathname.startsWith("/workspaces/") && page.url.pathname.endsWith("/settings")));
  const workspaceUrl = $derived(data.workspace ? `/workspaces/${encodeURIComponent(data.workspace.id)}/settings` : "/_old/workspaces");
  async function selectWorkspace(id: string) {
    sidebarOpen = false;
    await goto(title === "Settings" ? `/workspaces/${encodeURIComponent(id)}/settings` : title === "Operations" ? `/operations?workspace=${encodeURIComponent(id)}` : title === "Activity" ? `/activity?workspace=${encodeURIComponent(id)}` : `/?workspace=${encodeURIComponent(id)}`);
  }
</script>
<a class="skip" href="#main-content">Skip to content</a>
<div class:configuration-open={configurationOpen} class="workspace-shell">
  <aside class:mobile-open={sidebarOpen} aria-label="Workspace navigation">
    <a class="brand" href="/" aria-label="Ayni Orchestrator home"><span class="brand-mark" aria-hidden="true"></span><span>ayni<small>ORCHESTRATOR</small></span></a>
    <Dropdown.Root>
      <Dropdown.Trigger class="workspace-switcher">
        <span class="workspace-avatar">{data.workspace?.displayName.slice(0, 1).toUpperCase() ?? "A"}</span>
        <span class="workspace-name">{data.workspace?.displayName ?? "Choose workspace"}<small>{data.workspace?.repositoryConnectionIds.length ?? 0} {data.workspace?.repositoryConnectionIds.length === 1 ? "repository" : "repositories"}</small></span>
        <ChevronDown size={15} aria-hidden="true" />
      </Dropdown.Trigger>
      <Dropdown.Content align="start" class="min-w-64">
        <Dropdown.Label>Workspaces</Dropdown.Label>
        {#each data.workspaces as workspace (workspace.id)}
          <Dropdown.Item onclick={() => selectWorkspace(workspace.id)}>{workspace.displayName}{#if workspace.id === data.workspace?.id}<Check class="ml-auto size-4" />{/if}</Dropdown.Item>
        {/each}
        <Dropdown.Separator />
        <Dropdown.Item onclick={() => goto("/_old/workspaces")}><Plus size={15} /> Create workspace</Dropdown.Item>
      </Dropdown.Content>
    </Dropdown.Root>
    <p class="section-label">Workspace</p>
    <nav><a class={title === "Operations" ? "nav-item selected" : "nav-item"} href={data.workspace ? `/operations?workspace=${encodeURIComponent(data.workspace.id)}` : "/operations"} aria-current={title === "Operations" ? "page" : undefined}><LayoutDashboard size={19} />Operations</a><a class={title === "Projects" ? "nav-item selected" : "nav-item"} href={data.workspace ? `/?workspace=${encodeURIComponent(data.workspace.id)}` : "/"} aria-current={title === "Projects" ? "page" : undefined}><Layers size={19} />Projects <span>{data.projects.length}</span></a><a class={title === "Activity" ? "nav-item selected" : "nav-item"} href={data.workspace ? `/activity?workspace=${encodeURIComponent(data.workspace.id)}` : "/activity"} aria-current={title === "Activity" ? "page" : undefined}><Activity size={19} />Activity</a></nav>
    <div class="sidebar-bottom"><button class="settings-link" class:selected={configurationOpen} onclick={() => { configurationOpen = !configurationOpen; sidebarOpen = false; }} aria-expanded={configurationOpen} aria-controls="configuration-navigation"><Settings2 size={17} />Configuration</button></div>
  </aside>
  {#if sidebarOpen}<button class="mobile-backdrop" onclick={() => sidebarOpen = false} aria-label="Close workspace navigation"></button>{/if}
  {#if configurationOpen}
    <nav id="configuration-navigation" class="configuration-navigation" aria-label="Configuration navigation">
      <div class="configuration-heading"><h2>Configuration</h2><button onclick={() => configurationOpen = false} aria-label="Close configuration navigation"><X size={17} /></button></div>
      <p class="configuration-group">Workspace</p>
      <a href={workspaceUrl}><Settings2 size={17} />Settings</a>
      <a href={`${workspaceUrl}?tab=repositories`}><Layers size={17} />Repositories</a>
      <p class="configuration-group">General</p>
      <a href="/settings/source-control" class:active={page.url.pathname.startsWith('/settings/source-control')} aria-current={page.url.pathname.startsWith('/settings/source-control')?'page':undefined}><GitBranch size={17} />Source control</a>
      <a href="/settings/container-registries" class:active={page.url.pathname.startsWith('/settings/container-registries')} aria-current={page.url.pathname.startsWith('/settings/container-registries')?'page':undefined}><Container size={17} />Container registries</a>
      <a href="/settings/llm-providers" class:active={page.url.pathname.startsWith('/settings/llm-providers')} aria-current={page.url.pathname.startsWith('/settings/llm-providers')?'page':undefined}><BrainCircuit size={17} />LLM providers</a>
      <a href="/settings/ayni" class:active={page.url.pathname.startsWith('/settings/ayni')} aria-current={page.url.pathname.startsWith('/settings/ayni')?'page':undefined}><AyniIcon size={17} />Ayni</a>
    </nav>
  {/if}
  <div class="main-shell">
    <header class="topbar"><button class="mobile-toggle" onclick={() => sidebarOpen = !sidebarOpen} aria-label="Toggle workspace navigation" aria-expanded={sidebarOpen}>{#if sidebarOpen}<PanelLeftClose size={20} />{:else}<Menu size={20} />{/if}</button><div class="breadcrumb"><span>{page.url.pathname.startsWith('/settings') ? 'Configuration' : data.workspace?.displayName ?? 'Workspace'}</span><span class="separator">/</span><strong>{title}</strong></div><ThemeToggle /><Dropdown.Root><Dropdown.Trigger class="inline-flex size-9 items-center justify-center hover:bg-accent" aria-label="Workspace actions" disabled={!data.workspace}><EllipsisVertical size={19} /></Dropdown.Trigger><Dropdown.Content align="end"><Dropdown.Item onclick={() => goto(workspaceUrl)}><Settings2 size={16} />Workspace settings</Dropdown.Item></Dropdown.Content></Dropdown.Root></header>
    <main id="main-content">{@render children()}</main>
  </div>
</div>

<style>
  .workspace-shell{display:grid;grid-template-columns:240px minmax(0,1fr);min-height:100vh}.workspace-shell.configuration-open{grid-template-columns:240px 260px minmax(0,1fr)}aside{position:sticky;top:0;height:100vh;background:var(--sidebar);border-right:1px solid var(--border);padding:30px 18px 16px;display:flex;flex-direction:column;gap:0}.brand{color:var(--primary);display:flex;align-items:center;gap:12px;margin:0 12px 36px;text-decoration:none;font-size:28px;font-weight:650;letter-spacing:-1px}.brand-mark{width:33px;height:33px;background:var(--primary);mask:url("/ayni-orchestrator.svg") center/contain no-repeat;flex-shrink:0}.brand small{display:block;font:9px var(--font-mono);letter-spacing:2px;color:var(--muted-foreground);margin-top:2px}.workspace-name{min-width:0;flex:1;text-align:left;overflow-wrap:anywhere}.workspace-name small{display:block;color:var(--muted-foreground);font-size:11px;margin-top:5px;font-weight:400}.workspace-avatar{width:34px;height:34px;display:grid;place-items:center;background:color-mix(in oklab,var(--primary) 15%,var(--sidebar));color:var(--primary);border-radius:var(--radius);font:13px var(--font-mono);flex-shrink:0}:global(.workspace-switcher){display:flex;align-items:center;gap:11px;width:100%;padding:14px 12px;border:1px solid var(--border);border-radius:var(--radius);background:var(--card);font-size:13px;cursor:pointer}.section-label{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin:30px 12px 14px}.nav-item{display:flex;align-items:center;gap:12px;padding:12px;border-radius:var(--radius);font-size:13px}.selected{background:color-mix(in oklab,var(--primary) 12%,var(--sidebar));color:var(--foreground)}.selected :global(svg){color:var(--primary)}.nav-item>span{margin-left:auto;font:10px var(--font-mono);color:var(--muted-foreground)}.sidebar-bottom{margin-top:auto;padding-top:30px}.settings-link{display:flex;width:100%;align-items:center;gap:10px;padding:12px 10px;font-size:11px;color:var(--muted-foreground);text-align:left}.settings-link:hover,.settings-link.selected{color:var(--foreground)}.configuration-navigation{position:sticky;top:0;height:100vh;overflow-y:auto;padding:28px 14px;background:var(--card);border-right:1px solid var(--border)}.configuration-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;padding:0 10px 16px}.configuration-heading h2{font-size:17px;font-weight:600}.configuration-heading button{display:grid;place-items:center;width:28px;height:28px;color:var(--muted-foreground)}.configuration-navigation a{display:flex;align-items:center;gap:11px;padding:11px 10px;border-radius:var(--radius);font-size:12px;color:var(--muted-foreground)}.configuration-navigation a:hover{background:var(--accent);color:var(--foreground)}.configuration-group{margin:15px 10px 7px;font:10px var(--font-mono);text-transform:uppercase;letter-spacing:1px;color:var(--muted-foreground)}.main-shell{min-width:0}.topbar{height:76px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between;padding:0 40px;gap:15px}.breadcrumb{display:flex;align-items:center;gap:18px;font-size:12px;color:var(--muted-foreground)}.breadcrumb strong{font-weight:400;color:var(--foreground)}.separator{opacity:.5}main{padding:40px;max-width:1560px;margin:0 auto}.skip{position:fixed;top:-100px;left:20px;z-index:100;background:var(--primary);color:var(--primary-foreground);padding:12px}.skip:focus{top:10px}.mobile-toggle,.mobile-backdrop{display:none}@media(max-width:1200px){main{padding:32px}.topbar{padding:0 32px}}@media(max-width:800px){.workspace-shell,.workspace-shell.configuration-open{grid-template-columns:1fr}aside{display:none}aside.mobile-open{display:flex;position:fixed;left:0;top:0;width:240px;z-index:50}.mobile-backdrop{display:block;position:fixed;inset:0;background:#0008;z-index:40}.configuration-navigation{position:fixed;left:0;top:64px;bottom:0;height:auto;width:min(300px,100vw);z-index:45;box-shadow:6px 0 20px #0002}.mobile-toggle{display:block}.topbar{height:64px;padding:0 22px}.breadcrumb{margin-right:auto}main{padding:30px 22px}}@media(max-width:540px){.breadcrumb{font-size:11px;gap:12px}}.configuration-navigation a.active{background:color-mix(in oklab,var(--primary) 12%,var(--card));color:var(--foreground)}</style>
