<script lang="ts">
  import type { Snippet } from "svelte";
  import type { WorkspaceData } from "$lib/workspace-types";
  import { afterNavigate } from "$app/navigation";
  import { page } from "$app/state";
  import { Layers, Settings2, PanelLeftClose, Menu, X, GitBranch, Package, BrainCircuit } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  let { data, title = "Projects", children }: { data: WorkspaceData; title?: string; children: Snippet } = $props();
  let sidebarOpen = $state(false);
  let configurationOpen = $state(page.url.pathname.startsWith("/settings"));
  afterNavigate(() => {
    sidebarOpen = false;
    configurationOpen = page.url.pathname.startsWith('/settings');
  });
</script>
<a class="skip" href="#main-content">Skip to content</a>
<div class:configuration-open={configurationOpen} class="workspace-shell">
  <aside class:mobile-open={sidebarOpen} aria-label="Workspace navigation">
    <a class="brand" href="/" aria-label="Ayni Orchestrator home"><span class="brand-mark" aria-hidden="true"></span><span>ayni<small>ORCHESTRATOR</small></span></a>
    <p class="section-label">Workspaces</p>
    <Button variant="ghost" href="/" class="w-full justify-start gap-3" aria-current={title==='Workspaces'?'page':undefined}><Layers size={19}/>All workspaces</Button>
    {#if data.workspace}
      <p class="section-label">{data.workspace.displayName}</p>
      <Button variant="ghost" href={`/?workspace=${encodeURIComponent(data.workspace.id)}`} class="w-full justify-start gap-3" aria-current={title==='Projects'?'page':undefined}><Layers size={19}/>Projects</Button>
    {/if}
    <div class="sidebar-bottom">
      <p class="section-label">Settings</p>
      <Button variant="ghost" href="/repositories" class="w-full justify-start gap-3" aria-current={title==='Repositories'?'page':undefined}><GitBranch size={19}/>Repositories</Button>
      <Button variant="ghost" class={configurationOpen ? "settings-link selected" : "settings-link"} onclick={() => { configurationOpen = !configurationOpen; sidebarOpen = false; }} aria-expanded={configurationOpen} aria-controls="configuration-navigation"><Settings2 size={19} />Configuration</Button>
    </div>
  </aside>
  {#if sidebarOpen}<Button variant="ghost" class="mobile-backdrop" onclick={() => sidebarOpen = false} aria-label="Close workspace navigation"></Button>{/if}
  {#if configurationOpen}
    <nav id="configuration-navigation" class="configuration-navigation" aria-label="Configuration navigation">
      <div class="configuration-heading"><h2>Configuration</h2><Button variant="ghost" onclick={() => configurationOpen = false} aria-label="Close configuration navigation"><X size={17} /></Button></div>
      <p class="configuration-group">Integrations</p>
      <a href="/settings/source-control" class:active={page.url.pathname.startsWith('/settings/source-control')} aria-current={page.url.pathname.startsWith('/settings/source-control')?'page':undefined}><GitBranch size={17} />Source control</a>
      <a href="/settings/container-registries" class:active={page.url.pathname.startsWith('/settings/container-registries')}><Package size={17} />Container registries</a>
      <a href="/settings/llm-providers" class:active={page.url.pathname.startsWith('/settings/llm-providers')}><BrainCircuit size={17} />LLM providers</a>
    </nav>
  {/if}
  <div class="main-shell">
    <header class="topbar"><Button variant="ghost" class="mobile-toggle" onclick={() => sidebarOpen = !sidebarOpen} aria-label="Toggle workspace navigation" aria-expanded={sidebarOpen}>{#if sidebarOpen}<PanelLeftClose size={20} />{:else}<Menu size={20} />{/if}</Button><div class="breadcrumb"><span>{page.url.pathname.startsWith('/settings') ? 'Configuration' : data.workspace?.displayName ?? 'Workspace'}</span><span class="separator">/</span><strong>{title}</strong></div><ThemeToggle /></header>
    <main id="main-content">{@render children()}</main>
  </div>
</div>

<style>
  .workspace-shell{display:grid;grid-template-columns:240px minmax(0,1fr);min-height:100vh}.workspace-shell.configuration-open{grid-template-columns:240px 260px minmax(0,1fr)}aside{position:sticky;top:0;height:100vh;background:var(--sidebar);border-right:1px solid var(--border);padding:30px 18px 16px;display:flex;flex-direction:column;gap:0}.brand{color:var(--primary);display:flex;align-items:center;gap:12px;margin:0 12px 36px;text-decoration:none;font-size:28px;font-weight:650;letter-spacing:-1px}.brand-mark{width:33px;height:33px;background:var(--primary);mask:url("/ayni-orchestrator.svg") center/contain no-repeat;flex-shrink:0}.brand small{display:block;font:9px var(--font-mono);letter-spacing:2px;color:var(--muted-foreground);margin-top:2px}.section-label{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin:30px 12px 14px}:global(.settings-link.selected){background:var(--accent);color:var(--foreground)}.sidebar-bottom{margin-top:auto;padding-top:30px}:global(.settings-link){display:flex;justify-content:flex-start;height:auto;width:100%;align-items:center;gap:10px;padding:12px 10px;font-size:14px;text-align:left}:global(.settings-link:hover),:global(.settings-link.selected){color:var(--foreground)}.configuration-navigation{position:sticky;top:0;height:100vh;overflow-y:auto;padding:28px 14px;background:var(--card);border-right:1px solid var(--border)}.configuration-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;padding:0 10px 16px}.configuration-heading h2{font-size:17px;font-weight:600}.configuration-heading :global(button){display:grid;place-items:center;width:28px;height:28px;color:var(--muted-foreground)}.configuration-navigation a{display:flex;align-items:center;gap:11px;padding:11px 10px;border-radius:var(--radius);font-size:12px;color:var(--muted-foreground)}.configuration-navigation a:hover{background:var(--accent);color:var(--foreground)}.configuration-group{margin:15px 10px 7px;font:10px var(--font-mono);text-transform:uppercase;letter-spacing:1px;color:var(--muted-foreground)}.main-shell{min-width:0}.topbar{height:76px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between;padding:0 40px;gap:15px}.breadcrumb{display:flex;align-items:center;gap:18px;font-size:12px;color:var(--muted-foreground)}.breadcrumb strong{font-weight:400;color:var(--foreground)}.separator{opacity:.5}main{padding:40px;max-width:1560px;margin:0 auto}.skip{position:fixed;top:-100px;left:20px;z-index:100;background:var(--primary);color:var(--primary-foreground);padding:12px}.skip:focus{top:10px}:global(.mobile-toggle),:global(.mobile-backdrop){display:none}@media(max-width:1200px){main{padding:32px}.topbar{padding:0 32px}}@media(max-width:800px){.workspace-shell,.workspace-shell.configuration-open{grid-template-columns:1fr}aside{display:none}aside.mobile-open{display:flex;position:fixed;left:0;top:0;width:240px;z-index:50}:global(.mobile-backdrop){display:block;position:fixed;inset:0;width:100vw;height:100dvh;padding:0;border:0;border-radius:0;background:#0008;z-index:40}.configuration-navigation{position:fixed;left:0;top:64px;bottom:0;height:auto;width:min(300px,100vw);z-index:45;box-shadow:6px 0 20px #0002}:global(.mobile-toggle){display:block}.topbar{height:64px;padding:0 22px}.breadcrumb{margin-right:auto}main{padding:30px 22px}}@media(max-width:540px){.breadcrumb{font-size:11px;gap:12px}}.configuration-navigation a.active{background:color-mix(in oklab,var(--primary) 12%,var(--card));color:var(--foreground)}</style>
