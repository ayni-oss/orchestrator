<script lang="ts">
 import { page } from '$app/state';
 import PageHeader from '$lib/components/page-header.svelte';
 import ProjectRepositories from '$lib/components/project-repositories.svelte';
 import Feedback from '$lib/components/feedback.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import type { PageProps } from './$types';
 let {data}:PageProps=$props();
 const project=$derived(data.projects.find(p=>p.id===page.params.id));
</script>
<PageHeader title={project?`${project.displayName} repositories`:'Project repositories'} description="Choose which shared repositories this project uses." />
<div class="mx-auto max-w-5xl space-y-6">
 {#if data.workspace&&page.params.id}{#key `${data.workspace.id}/${page.params.id}`}<ProjectRepositories workspaceId={data.workspace.id} projectId={page.params.id}/>{/key}
 {:else}<Feedback kind="error" title="Choose a workspace" message={data.error||'Open a project from its workspace to manage repository associations.'}/>{/if}
 <Button variant="outline" href={data.workspace?`/?workspace=${data.workspace.id}`:'/'}>Back to projects</Button>
</div>
