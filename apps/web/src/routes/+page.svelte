<script lang="ts">
  import type { PageProps } from "./$types";
  import { Layers, Search, ArrowUpRight, FolderGit2 } from "@lucide/svelte";
  import WorkspaceShell from "$lib/components/workspace-shell.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import CreateProject from "$lib/components/create-project.svelte";
  import CreateWorkspaceDialog from "$lib/components/create-workspace-dialog.svelte";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  let { data }: PageProps = $props();
  let search = $state("");
  const projects = $derived(data.projects.filter(project => `${project.displayName} ${project.goal}`.toLowerCase().includes(search.toLowerCase())));
  const projectUrl = (id: string) => `/projects/${encodeURIComponent(id)}`;
</script>
<svelte:head><title>{data.workspace ? `${data.workspace.displayName} · Projects` : "Projects"} · Ayni Orchestrator</title></svelte:head>
<ApplicationNotifications />
<WorkspaceShell {data}>
      <PageHeader eyebrow="Workspace / Projects" title="Projects" description="Give your work a goal. Build the plan from there.">
        {#snippet actions()}{#if data.workspace?.state === "active"}{#key data.workspace.id}<CreateProject {data} label="New project" />{/key}{:else if !data.error}<CreateWorkspaceDialog showTrigger />{/if}{/snippet}
      </PageHeader>
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

      {:else}<section class="empty-state"><Layers size={32} /><h2>A workspace for your projects</h2><p>Create a workspace and connect repositories to get started.</p></section>{/if}

</WorkspaceShell>
<style>.project-toolbar{display:flex;align-items:center;justify-content:space-between;gap:20px;margin-bottom:22px}.project-toolbar>p{font-size:12px}.project-toolbar>p span{margin-left:9px;color:var(--muted-foreground);font-size:11px}.search-field{position:relative;width:245px}.search-field>:global(svg){position:absolute;left:11px;top:10px;color:var(--muted-foreground)}.search-field :global(input){padding-left:35px;font-size:12px}.project-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:22px}.project-card{display:flex;flex-direction:column;min-height:285px;padding:26px;background:linear-gradient(140deg,color-mix(in oklab,var(--primary) 5%,var(--card)),var(--card));border:1px solid var(--border);border-radius:var(--radius);box-shadow:var(--shadow-sm);transition:border-color .18s,transform .18s;text-decoration:none}.project-card:hover{border-color:color-mix(in oklab,var(--primary) 60%,var(--border));transform:translateY(-3px)}.card-top{display:flex;align-items:center;justify-content:space-between;margin-bottom:27px}.project-symbol{font:38px var(--font-sans);color:var(--primary);line-height:1}.card-top :global(.open-icon){color:var(--muted-foreground);opacity:.5}.project-card h2{font-size:20px;letter-spacing:-.5px;font-weight:550;overflow-wrap:anywhere}.project-goal{font-size:12px;line-height:1.9;color:var(--muted-foreground);margin:13px 0 25px;display:-webkit-box;line-clamp:3;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}.project-card footer{display:flex;align-items:center;justify-content:space-between;gap:10px;padding-top:17px;margin-top:auto;border-top:1px solid var(--border);font:9px var(--font-mono);color:var(--muted-foreground)}.project-card footer>span{display:flex;align-items:center;gap:7px}.empty-state{padding:70px 24px;display:flex;align-items:center;flex-direction:column;text-align:center;gap:18px;border:1px dashed var(--border);border-radius:var(--radius);color:var(--muted-foreground)}.empty-state h2{font-size:20px;color:var(--foreground)}.empty-state p{font-size:13px;max-width:400px;line-height:1.8}.notice{padding:25px;border:1px solid var(--border);border-radius:var(--radius);margin-bottom:20px;font-size:13px}.notice h2{font-size:18px;margin-bottom:10px}.notice p{margin-bottom:20px;color:var(--muted-foreground)}@media(min-width:1500px){.project-card{min-height:320px}}@media(max-width:1200px){.project-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:800px){}@media(max-width:540px){.project-grid{grid-template-columns:1fr}.project-toolbar{align-items:stretch;flex-direction:column}.search-field{width:100%}.project-card{min-height:260px}.project-toolbar>p span{display:none}}@media(prefers-reduced-motion:reduce){.project-card{transition:none}}</style>
