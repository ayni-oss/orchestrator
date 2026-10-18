<script lang="ts">
  import type { Snippet } from "svelte";
  import type { WorkspaceDto, ProjectDto } from "@ayni-orchestrator/bff/server";
  import { goto } from "$app/navigation";
  import { Layers, ChevronDown, Settings2, ArrowUpRight, Plus, Check, PanelLeftClose, Menu, EllipsisVertical } from "@lucide/svelte";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import * as Dropdown from "$lib/components/ui/dropdown-menu/index.js";
  let { data, title = "Projects", children }: { data: { workspace?: WorkspaceDto; workspaces: WorkspaceDto[]; projects: ProjectDto[] }; title?: string; children: Snippet } = $props();
  let sidebarOpen = $state(false);
  const workspaceUrl = $derived(data.workspace ? `/workspaces/${encodeURIComponent(data.workspace.id)}/settings` : "/_old/workspaces");
  const projectUrl = (id: string) => `/projects/${encodeURIComponent(id)}`;
  async function selectWorkspace(id: string) {
    sidebarOpen = false;
    await goto(title === "Settings" ? `/workspaces/${encodeURIComponent(id)}/settings` : `/?workspace=${encodeURIComponent(id)}`);
  }
</script>
<a class="skip" href="#main-content">Skip to content</a>
<div class="workspace-shell">
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
    <nav><a class={title === "Projects" ? "nav-item selected" : "nav-item"} href={data.workspace ? `/?workspace=${encodeURIComponent(data.workspace.id)}` : "/"} aria-current={title === "Projects" ? "page" : undefined}><Layers size={19} />Projects <span>{data.projects.length}</span></a></nav>
    <div class="sidebar-projects"><p class="section-label">Your projects</p>
      {#each data.projects.slice(0, 8) as project (project.id)}<a class="project-shortcut" href={projectUrl(project.id)}><i></i><span>{project.displayName}</span></a>{/each}
      {#if !data.projects.length}<p class="sidebar-empty">Your projects will appear here.</p>{/if}
    </div>
    <div class="sidebar-bottom"><a class="settings-link" href={workspaceUrl}><Settings2 size={17} />Workspace settings<ArrowUpRight size={13} /></a><a class="reference-link" href="/_old">Previous application <ArrowUpRight size={12} /></a><div class="sidebar-footer"><span>AYNI ORCHESTRATOR</span><ThemeToggle /></div></div>
  </aside>
  {#if sidebarOpen}<button class="mobile-backdrop" onclick={() => sidebarOpen = false} aria-label="Close workspace navigation"></button>{/if}
  <div class="main-shell">
    <header class="topbar"><button class="mobile-toggle" onclick={() => sidebarOpen = !sidebarOpen} aria-label="Toggle workspace navigation" aria-expanded={sidebarOpen}>{#if sidebarOpen}<PanelLeftClose size={20} />{:else}<Menu size={20} />{/if}</button><div class="breadcrumb"><span>{data.workspace?.displayName ?? "Workspace"}</span><span class="separator">/</span><strong>{title}</strong></div><Dropdown.Root><Dropdown.Trigger class="inline-flex size-9 items-center justify-center hover:bg-accent" aria-label="Workspace actions" disabled={!data.workspace}><EllipsisVertical size={19} /></Dropdown.Trigger><Dropdown.Content align="end"><Dropdown.Item onclick={() => goto(workspaceUrl)}><Settings2 size={16} />Workspace settings</Dropdown.Item></Dropdown.Content></Dropdown.Root></header>
    <main id="main-content">{@render children()}</main>
  </div>
</div>

<style>
  .workspace-shell{display:grid;grid-template-columns:240px minmax(0,1fr);min-height:100vh}aside{position:sticky;top:0;height:100vh;background:var(--sidebar);border-right:1px solid var(--border);padding:30px 18px 16px;display:flex;flex-direction:column;gap:0}.brand{color:var(--primary);display:flex;align-items:center;gap:12px;margin:0 12px 36px;text-decoration:none;font-size:28px;font-weight:650;letter-spacing:-1px}.brand-mark{width:33px;height:33px;background:var(--primary);mask:url("/ayni-orchestrator.svg") center/contain no-repeat;flex-shrink:0}.brand small{display:block;font:9px var(--font-mono);letter-spacing:2px;color:var(--muted-foreground);margin-top:2px}.workspace-name{min-width:0;flex:1;text-align:left;overflow-wrap:anywhere}.workspace-name small{display:block;color:var(--muted-foreground);font-size:11px;margin-top:5px;font-weight:400}.workspace-avatar{width:34px;height:34px;display:grid;place-items:center;background:color-mix(in oklab,var(--primary) 15%,var(--sidebar));color:var(--primary);border-radius:var(--radius);font:13px var(--font-mono);flex-shrink:0}:global(.workspace-switcher){display:flex;align-items:center;gap:11px;width:100%;padding:14px 12px;border:1px solid var(--border);border-radius:var(--radius);background:var(--card);font-size:13px;cursor:pointer}.section-label{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin:30px 12px 14px}.nav-item{display:flex;align-items:center;gap:12px;padding:12px;border-radius:var(--radius);font-size:13px}.selected{background:color-mix(in oklab,var(--primary) 12%,var(--sidebar));color:var(--foreground)}.selected :global(svg){color:var(--primary)}.nav-item>span{margin-left:auto;font:10px var(--font-mono);color:var(--muted-foreground)}.sidebar-projects{overflow:auto}.project-shortcut{display:flex;align-items:center;gap:10px;padding:10px 12px;font-size:11px;color:var(--muted-foreground)}.project-shortcut span{overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.project-shortcut:hover{color:var(--foreground)}.project-shortcut i,.project-state i{width:5px;height:5px;border-radius:50%;background:var(--primary);flex-shrink:0}.sidebar-empty{font-size:11px;line-height:1.8;color:var(--muted-foreground);padding:0 12px}.sidebar-bottom{margin-top:auto;padding-top:30px}.settings-link{display:flex;align-items:center;gap:10px;padding:12px 10px;font-size:11px;color:var(--muted-foreground)}.settings-link :global(svg:last-child){margin-left:auto}.reference-link{display:flex;align-items:center;gap:8px;margin:8px 10px 22px;font-size:10px;color:var(--muted-foreground)}.sidebar-footer{display:flex;align-items:center;justify-content:space-between;border-top:1px solid var(--border);padding-top:12px}.sidebar-footer>span{font:8px var(--font-mono);letter-spacing:1px;color:var(--muted-foreground)}.main-shell{min-width:0}.topbar{height:76px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between;padding:0 40px;gap:15px}.breadcrumb{display:flex;align-items:center;gap:18px;font-size:12px;color:var(--muted-foreground)}.breadcrumb strong{font-weight:400;color:var(--foreground)}.separator{opacity:.5}.scope-label{font:9px var(--font-mono);letter-spacing:1px;color:var(--muted-foreground)}main{padding:40px;max-width:1560px;margin:0 auto}.skip{position:fixed;top:-100px;left:20px;z-index:100;background:var(--primary);color:var(--primary-foreground);padding:12px}.skip:focus{top:10px}.mobile-toggle,.mobile-backdrop{display:none}@media(max-width:1200px){main{padding:32px}.topbar{padding:0 32px}}@media(max-width:800px){.workspace-shell{grid-template-columns:1fr}aside{display:none}aside.mobile-open{display:flex;position:fixed;left:0;top:0;width:240px;z-index:50}.mobile-backdrop{display:block;position:fixed;inset:0;background:#0008;z-index:40}.mobile-toggle{display:block}.topbar{height:64px;padding:0 22px}.scope-label{display:none}.breadcrumb{margin-right:auto}main{padding:30px 22px}}@media(max-width:540px){.breadcrumb{font-size:11px;gap:12px}}</style>