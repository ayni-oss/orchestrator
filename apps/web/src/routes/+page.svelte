<script lang="ts">
 import type { PageProps } from './$types';
 import { Layers } from '@lucide/svelte';
 import PageHeader from '$lib/components/page-header.svelte';
 import HierarchyCard from '$lib/components/hierarchy-card.svelte';
 import CreateProject from '$lib/components/create-project.svelte';
 import CreateWorkspaceDialog from '$lib/components/create-workspace-dialog.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import { Input } from '$lib/components/ui/input/index.js';
 let {data}:PageProps=$props();let search=$state('');
 const label=$derived(data.workspace?'Projects':'Workspaces');
 const count=$derived(data.workspace?data.projects.length:data.workspaces.length);
 const projects=$derived(data.projects.filter(project=>`${project.displayName} ${project.goal}`.toLowerCase().includes(search.toLowerCase())));
 const workspaces=$derived(data.workspaces.filter(workspace=>workspace.displayName.toLowerCase().includes(search.toLowerCase())));
</script>
<PageHeader eyebrow={data.workspace?.displayName??'Workspaces'} title={label} description={data.workspace?'Give your work a goal. Build the plan from there.':'Choose a workspace to explore its projects.'}>
 {#snippet actions()}{#if data.workspace}<CreateProject workspaceId={data.workspace.id}/>{:else}<CreateWorkspaceDialog showTrigger/>{/if}{/snippet}
</PageHeader>
{#if data.error}<section role="alert" class="space-y-4 rounded-xl border p-6"><h2>{label} couldn’t be loaded</h2><p>{data.error}</p><Button variant="outline" onclick={()=>location.reload()}>Try again</Button></section>
{:else}
 <div class="mb-6 flex flex-wrap items-center justify-between gap-4"><p class="text-sm text-muted-foreground">{count} {count===1?(data.workspace?'project':'workspace'):label.toLowerCase()}</p><Input class="max-w-xs" bind:value={search} aria-label={`Search ${label.toLowerCase()}`} placeholder={`Search ${label.toLowerCase()}…`}/></div>
 <div class="grid gap-5 sm:grid-cols-2 xl:grid-cols-3">
 {#if data.workspace}
  {#each projects as project (project.id)}<HierarchyCard kind="project" id={project.id} name={project.displayName} resourceVersion={project.resourceVersion} workspaceId={data.workspace.id} description={project.goal} href={`/projects/${project.id}/repositories?workspace=${data.workspace.id}`}/>{/each}
 {:else}
  {#each workspaces as workspace (workspace.id)}<HierarchyCard kind="workspace" id={workspace.id} name={workspace.displayName} resourceVersion={workspace.resourceVersion} href={`/?workspace=${workspace.id}`}/>{/each}
 {/if}
 </div>
 {#if data.workspace?!projects.length:!workspaces.length}<section class="mt-6 flex flex-col items-center gap-4 rounded-xl border border-dashed p-12 text-center text-muted-foreground"><Layers size={32}/><h2 class="text-lg text-foreground">{search?'No matches':`Create your first ${data.workspace?'project':'workspace'}`}</h2></section>{/if}
 {#if data.workspace&&data.nextProject}<Button class="mt-6" variant="outline" href={`/?workspace=${data.workspace.id}&afterProject=${data.nextProject}`}>Next projects</Button>{/if}
 {#if !data.workspace&&data.nextWorkspace}<Button class="mt-6" variant="outline" href={`/?afterWorkspace=${data.nextWorkspace}`}>More workspaces</Button>{/if}
{/if}
