<script lang="ts">
  import {page} from "$app/state";
  import {goto} from "$app/navigation";
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import CreateProject from "$lib/components/create-project.svelte";
  import type {PageProps} from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import ManageWorkspaceRepositories from "$lib/components/manage-workspace-repositories.svelte";
  import RepositoryCard from "$lib/components/repository-card.svelte";
  import {Button} from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  let {data}:PageProps=$props();
  const view=$derived(["repositories","settings"].includes(page.url.searchParams.get("view")??"") ? page.url.searchParams.get("view")! : "projects");
</script>
<svelte:head><title>{data.workspace?.displayName??"Workspace unavailable"} · Ayni Orchestrator</title></svelte:head>
{#if data.workspace}
  <PageHeader eyebrow={data.workspace.state==="archived"?"Archived workspace":"Workspace"} title={data.workspace.displayName} description="Shared repositories and agent defaults for goal-oriented projects." />
  {#if data.workspace.state==="archived"}<p>New work is disabled. Existing tasks and project history are retained.</p>{/if}
  <Tabs.Root value={view} onValueChange={value=>{void goto(`?view=${value}`,{keepFocus:true,noScroll:true});}} class="gap-6">
    <Tabs.List aria-label="Workspace views">
      <Tabs.Trigger value="projects">Projects</Tabs.Trigger>
      <Tabs.Trigger value="repositories">Repositories</Tabs.Trigger>
      <Tabs.Trigger value="settings">Settings</Tabs.Trigger>
    </Tabs.List>
  <Tabs.Content value="projects">
  <section class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-4">
      <h2 class="text-lg font-semibold">Projects</h2>
      {#if data.workspace.state==="active"}{#key data.workspace.id}<CreateProject {data} />{/key}{/if}
    </div>
    <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
    {#each data.projects as project (project.id)}<Card.Root><Card.Header><Card.Title><a class="hover:underline" href={`/_old/projects/${encodeURIComponent(project.id)}`}>{project.displayName}</a></Card.Title><Card.Description>{project.goal}</Card.Description></Card.Header><Card.Content>{project.state}</Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No projects yet. Create a project to define a goal and start work.</p>{/each}
  </div>{#if data.nextPageToken}<Button variant="outline" href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`}>More projects</Button>{/if}</section>
  </Tabs.Content>
  <Tabs.Content value="repositories">
  <section class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-4">
      <h2 class="text-lg font-semibold">Repositories</h2>
      {#if data.workspace.state==="active"}{#key data.workspace.id}<ManageWorkspaceRepositories {data} />{/key}{/if}
    </div>
    <p class="text-sm text-muted-foreground">Projects can use these repositories. Runtime configuration is shared wherever a repository is used. Connect new repositories in the <a href="/_old/repositories" class="text-foreground underline underline-offset-4 hover:text-primary">repository catalog</a>.</p>
    <div class="grid gap-4 lg:grid-cols-2">
      {#each data.repositorySummaries as summary (summary.repository.id)}
        <RepositoryCard {summary} />
      {/each}
      {#each data.workspace.repositoryConnectionIds.filter(id=>!data.repositorySummaries.some(summary=>summary.repository.id===id)) as id (id)}
        <Card.Root><Card.Header><Card.Title><a class="hover:underline" href={`/_old/repositories/${encodeURIComponent(id)}`}>{id}</a></Card.Title><Card.Description>Repository details are unavailable.</Card.Description></Card.Header></Card.Root>
      {/each}
    </div>
  </section>
  </Tabs.Content>
  <Tabs.Content value="settings">
    <Button href={`/workspaces/${encodeURIComponent(data.workspace.id)}/settings`}>Open workspace settings</Button>
  </Tabs.Content>
  </Tabs.Root>
{/if}
