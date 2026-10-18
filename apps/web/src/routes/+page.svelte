<script lang="ts">
  import type { PageProps } from "./$types";
  import { goto } from "$app/navigation";
  import { Layers, ChevronDown, Search, Settings2, ArrowUpRight, FolderGit2, Plus, Check, PanelLeftClose, Menu } from "@lucide/svelte";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import CreateProject from "$lib/components/create-project.svelte";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import * as Dropdown from "$lib/components/ui/dropdown-menu/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  let { data }: PageProps = $props();
  let search = $state("");
  let sidebarOpen = $state(false);
  const projects = $derived(data.projects.filter(project => `${project.displayName} ${project.goal}`.toLowerCase().includes(search.toLowerCase())));
  const workspaceUrl = $derived(data.workspace ? `/_old/workspaces/${encodeURIComponent(data.workspace.id)}` : "/_old/workspaces");
  const projectUrl = (id: string) => `/_old/projects/${encodeURIComponent(id)}`;
  async function selectWorkspace(id: string) {
    search = "";
    sidebarOpen = false;
    await goto(`/?workspace=${encodeURIComponent(id)}`);
  }
</script>

<svelte:head><title>{data.workspace ? `${data.workspace.displayName} · Projects` : "Projects"} · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<a class="skip" href="#projects">Skip to projects</a>
<div class="workspace-shell">
  <aside class:mobile-open={sidebarOpen} aria-label="Workspace navigation">
    <a class="brand" href="/" aria-label="Ayni Orchestrator home"><img src="/ayni-orchestrator.svg" alt="" /><span>ayni<small>ORCHESTRATOR</small></span></a>
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
        <Dropdown.Item onclick={() => goto(`${workspaceUrl}?view=settings`)}>Manage current workspace</Dropdown.Item>
        <Dropdown.Item onclick={() => goto("/_old/workspaces")}><Plus size={15} /> Create workspace</Dropdown.Item>
      </Dropdown.Content>
    </Dropdown.Root>
    <p class="section-label">Workspace</p>
    <nav><a class="nav-item selected" href={data.workspace ? `/?workspace=${encodeURIComponent(data.workspace.id)}` : "/"} aria-current="page"><Layers size={19} />Projects <span>{data.projects.length}</span></a></nav>
    <div class="sidebar-projects"><p class="section-label">Your projects</p>
      {#each data.projects.slice(0, 8) as project (project.id)}<a class="project-shortcut" href={projectUrl(project.id)}><i></i><span>{project.displayName}</span></a>{/each}
      {#if !data.projects.length}<p class="sidebar-empty">Your projects will appear here.</p>{/if}
    </div>
    <div class="sidebar-bottom"><a class="settings-link" href={`${workspaceUrl}?view=settings`}><Settings2 size={17} />Workspace settings<ArrowUpRight size={13} /></a><a class="reference-link" href="/_old">Previous application <ArrowUpRight size={12} /></a><div class="sidebar-footer"><span>AYNI ORCHESTRATOR</span><ThemeToggle /></div></div>
  </aside>
  {#if sidebarOpen}<button class="mobile-backdrop" onclick={() => sidebarOpen = false} aria-label="Close workspace navigation"></button>{/if}
  <div class="main-shell">
    <header class="topbar"><button class="mobile-toggle" onclick={() => sidebarOpen = !sidebarOpen} aria-label="Toggle workspace navigation" aria-expanded={sidebarOpen}>{#if sidebarOpen}<PanelLeftClose size={20} />{:else}<Menu size={20} />{/if}</button><div class="breadcrumb"><span>{data.workspace?.displayName ?? "Workspace"}</span><span class="separator">/</span><strong>Projects</strong></div><span class="scope-label">WORKSPACE VIEW</span></header>
    <main id="projects">
      <div class="page-heading"><div><p class="eyebrow">Workspace / Projects</p><h1>Projects</h1><p class="subtitle">Give your work a goal. Build the plan from there.</p></div>{#if data.workspace?.state === "active"}{#key data.workspace.id}<CreateProject {data} label="New project" />{/key}{:else if !data.error}<Button href="/_old/workspaces"><Plus size={16} />Create workspace</Button>{/if}</div>
      {#if data.error}<section class="notice" role="alert"><h2>Projects couldn’t be loaded</h2><p>{data.error.message}</p><Button variant="outline" onclick={() => location.reload()}>Try again</Button></section>
      {:else if data.workspace}
        {#if data.workspace.state !== "active"}<p class="notice">This workspace is archived. Its projects remain available for reference.</p>{/if}
        <div class="project-toolbar"><p>{data.projects.length} {data.projects.length === 1 ? "project" : "projects"}<span>in this workspace</span></p><div class="search-field"><Search size={16} aria-hidden="true" /><Input bind:value={search} aria-label="Search projects" placeholder="Search projects…" /></div></div>
        <div class="project-grid">
          {#each projects as project, index (project.id)}
            <a class="project-card" href={projectUrl(project.id)}>
              <div class="card-top"><span class="project-symbol" aria-hidden="true">{["◇", "⬡", "◈"][index % 3]}</span><ArrowUpRight class="open-icon" size={18} aria-hidden="true" /></div>
              <h2>{project.displayName}</h2><p class="project-goal">{project.goal || "No project goal yet."}</p>
              <footer><span><FolderGit2 size={14} />{project.repositoryConnectionIds.length} {project.repositoryConnectionIds.length === 1 ? "repository" : "repositories"}</span><span class="project-state"><i></i>{project.state === "active" ? "Active" : "Archived"}</span></footer>
            </a>
          {/each}
        </div>
        {#if !projects.length}<section class="empty-state"><Layers size={32} /><h2>{search ? "No matching projects" : "Start with one clear goal"}</h2><p>{search ? "Try a different project name or goal." : "Create a project to bring your repositories, conversations, and plans together."}</p>{#if search}<Button variant="outline" onclick={() => search = ""}>Clear search</Button>{/if}</section>{/if}
        <p class="view-note">Project details open in the previous application while we build the new experience.</p>
      {:else}<section class="empty-state"><Layers size={32} /><h2>A workspace for your projects</h2><p>Create a workspace and connect repositories to get started.</p></section>{/if}
    </main>
  </div>
</div>

<style>
  .workspace-shell{display:grid;grid-template-columns:240px minmax(0,1fr);min-height:100vh}aside{position:sticky;top:0;height:100vh;background:var(--sidebar);border-right:1px solid var(--border);padding:30px 18px 16px;display:flex;flex-direction:column;gap:0}.brand{display:flex;align-items:center;gap:12px;margin:0 12px 36px;text-decoration:none;font-size:28px;font-weight:650;letter-spacing:-1px}.brand img{width:33px;height:33px}.brand small{display:block;font:9px var(--font-mono);letter-spacing:2px;color:var(--muted-foreground);margin-top:2px}.workspace-name{min-width:0;flex:1;text-align:left;overflow-wrap:anywhere}.workspace-name small{display:block;color:var(--muted-foreground);font-size:11px;margin-top:5px;font-weight:400}.workspace-avatar{width:34px;height:34px;display:grid;place-items:center;background:color-mix(in oklab,var(--primary) 15%,var(--sidebar));color:var(--primary);border-radius:var(--radius);font:13px var(--font-mono);flex-shrink:0}:global(.workspace-switcher){display:flex;align-items:center;gap:11px;width:100%;padding:14px 12px;border:1px solid var(--border);border-radius:var(--radius);background:var(--card);font-size:13px;cursor:pointer}.section-label{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin:30px 12px 14px}.nav-item{display:flex;align-items:center;gap:12px;padding:12px;border-radius:var(--radius);font-size:13px}.selected{background:color-mix(in oklab,var(--primary) 12%,var(--sidebar));color:var(--foreground)}.selected :global(svg){color:var(--primary)}.nav-item>span{margin-left:auto;font:10px var(--font-mono);color:var(--muted-foreground)}.sidebar-projects{overflow:auto}.project-shortcut{display:flex;align-items:center;gap:10px;padding:10px 12px;font-size:11px;color:var(--muted-foreground)}.project-shortcut span{overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.project-shortcut:hover{color:var(--foreground)}.project-shortcut i,.project-state i{width:5px;height:5px;border-radius:50%;background:var(--primary);flex-shrink:0}.sidebar-empty{font-size:11px;line-height:1.8;color:var(--muted-foreground);padding:0 12px}.sidebar-bottom{margin-top:auto;padding-top:30px}.settings-link{display:flex;align-items:center;gap:10px;padding:12px 10px;font-size:11px;color:var(--muted-foreground)}.settings-link :global(svg:last-child){margin-left:auto}.reference-link{display:flex;align-items:center;gap:8px;margin:8px 10px 22px;font-size:10px;color:var(--muted-foreground)}.sidebar-footer{display:flex;align-items:center;justify-content:space-between;border-top:1px solid var(--border);padding-top:12px}.sidebar-footer>span{font:8px var(--font-mono);letter-spacing:1px;color:var(--muted-foreground)}.main-shell{min-width:0}.topbar{height:76px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between;padding:0 40px;gap:15px}.breadcrumb{display:flex;align-items:center;gap:18px;font-size:12px;color:var(--muted-foreground)}.breadcrumb strong{font-weight:400;color:var(--foreground)}.separator{opacity:.5}.scope-label{font:9px var(--font-mono);letter-spacing:1px;color:var(--muted-foreground)}main{padding:40px;max-width:1560px;margin:0 auto}.page-heading{display:flex;align-items:center;justify-content:space-between;gap:22px;margin-bottom:38px}.eyebrow{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin-bottom:14px}h1{font-size:36px;letter-spacing:-1.4px;font-weight:600;line-height:1.2}.subtitle{font-size:13px;color:var(--muted-foreground);margin-top:13px;line-height:1.7}.project-toolbar{display:flex;align-items:center;justify-content:space-between;gap:20px;margin-bottom:22px}.project-toolbar>p{font-size:12px}.project-toolbar>p span{margin-left:9px;color:var(--muted-foreground);font-size:11px}.search-field{position:relative;width:245px}.search-field>:global(svg){position:absolute;left:11px;top:10px;color:var(--muted-foreground)}.search-field :global(input){padding-left:35px;font-size:12px}.project-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:22px}.project-card{display:flex;flex-direction:column;min-height:285px;padding:26px;background:linear-gradient(140deg,color-mix(in oklab,var(--primary) 5%,var(--card)),var(--card));border:1px solid var(--border);border-radius:calc(var(--radius) + 4px);transition:border-color .18s,transform .18s;text-decoration:none}.project-card:hover{border-color:color-mix(in oklab,var(--primary) 60%,var(--border));transform:translateY(-3px)}.card-top{display:flex;align-items:center;justify-content:space-between;margin-bottom:27px}.project-symbol{font:38px var(--font-sans);color:var(--primary);line-height:1}.card-top :global(.open-icon){color:var(--muted-foreground);opacity:.5}.project-card h2{font-size:20px;letter-spacing:-.5px;font-weight:550;overflow-wrap:anywhere}.project-goal{font-size:12px;line-height:1.9;color:var(--muted-foreground);margin:13px 0 25px;display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}.project-card footer{display:flex;align-items:center;justify-content:space-between;gap:10px;padding-top:17px;margin-top:auto;border-top:1px solid var(--border);font:9px var(--font-mono);color:var(--muted-foreground)}.project-card footer>span{display:flex;align-items:center;gap:7px}.view-note{font-size:10px;color:var(--muted-foreground);margin-top:28px}.empty-state{padding:70px 24px;display:flex;align-items:center;flex-direction:column;text-align:center;gap:18px;border:1px dashed var(--border);border-radius:var(--radius);color:var(--muted-foreground)}.empty-state h2{font-size:20px;color:var(--foreground)}.empty-state p{font-size:13px;max-width:400px;line-height:1.8}.notice{padding:25px;border:1px solid var(--border);border-radius:var(--radius);margin-bottom:20px;font-size:13px}.notice h2{font-size:18px;margin-bottom:10px}.notice p{margin-bottom:20px;color:var(--muted-foreground)}.skip{position:fixed;top:-100px;left:20px;z-index:100;background:var(--primary);color:var(--primary-foreground);padding:12px}.skip:focus{top:10px}.mobile-toggle,.mobile-backdrop{display:none}@media(min-width:1500px){.project-card{min-height:320px}}@media(max-width:1200px){.project-grid{grid-template-columns:repeat(2,minmax(0,1fr))}main{padding:32px}.topbar{padding:0 32px}}@media(max-width:800px){.workspace-shell{grid-template-columns:1fr}aside{display:none}aside.mobile-open{display:flex;position:fixed;left:0;top:0;width:240px;z-index:50}.mobile-backdrop{display:block;position:fixed;inset:0;background:#0008;z-index:40}.mobile-toggle{display:block}.topbar{height:64px;padding:0 22px}.scope-label{display:none}.breadcrumb{margin-right:auto}main{padding:30px 22px}.page-heading{align-items:flex-start}h1{font-size:30px}}@media(max-width:540px){.project-grid{grid-template-columns:1fr}.project-toolbar{align-items:stretch;flex-direction:column}.search-field{width:100%}.page-heading{flex-wrap:wrap}.project-card{min-height:260px}.project-toolbar>p span{display:none}.breadcrumb{font-size:11px;gap:12px}}@media(prefers-reduced-motion:reduce){.project-card{transition:none}}
</style>
