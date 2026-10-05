<script lang="ts">
  import { enhance } from "$app/forms";
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  let { data, form }: PageProps = $props();
</script>

<svelte:head><title>Projects · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader eyebrow="Workspace" title="Projects" description="Each project contains one repository, its runtime, and its tasks." />
  {#if data.error}<Feedback kind="error" title="Projects unavailable" message={data.error.message} requestId={data.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Project not created" message={form.error.message} requestId={data.requestId} />{/if}
  <div class="grid gap-4 md:grid-cols-2">
    {#each data.projects as project (project.id)}
      <Card.Root><Card.Header><Card.Title><a href={`/projects/${encodeURIComponent(project.id)}`}>{project.displayName}</a></Card.Title><Card.Description>{project.state === "archived" ? "Archived · history retained" : "Active project"}</Card.Description></Card.Header></Card.Root>
    {:else}<p class="text-muted-foreground">Create your first project to organize work around a repository.</p>{/each}
  </div>
  {#if data.nextPageToken}<Button href={`/projects?pageToken=${encodeURIComponent(data.nextPageToken)}`} variant="outline">More projects</Button>{/if}
  <Card.Root><Card.Header><Card.Title>Create project</Card.Title><Card.Description>Select a connected repository. A repository can belong to only one active project.</Card.Description></Card.Header><Card.Content>
    <form use:enhance method="POST" action="?/create" class="max-w-xl space-y-4">
      <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
      <Label for="project-name">Project name</Label><Input id="project-name" name="displayName" required maxlength={256} />
      <Label for="project-repository">Repository</Label>
      <select id="project-repository" name="repositoryConnectionId" required class="h-10 w-full rounded-lg border bg-background px-3">
        <option value="">Select a repository</option>
        {#each data.repositories as repository (repository.id)}<option value={repository.id}>{repository.displayName}</option>{/each}
      </select>
      {#if data.repositoryPageToken}<a class="block underline" href={`/projects?repositoryPageToken=${encodeURIComponent(data.repositoryPageToken)}`}>More repository choices</a>{/if}
      <div class="flex gap-3"><Button type="submit" disabled={data.repositories.length === 0}>Create project</Button><Button href="/repositories" variant="outline">Manage repository connections</Button></div>
    </form>
  </Card.Content></Card.Root>
</div>
