<script lang="ts">
  import { enhance } from "$app/forms";
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { words } from "$lib/presentation.js";
  let { data, form }: PageProps = $props();
</script>

<svelte:head><title>{data.project?.displayName ?? "Project unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <Button href="/projects" variant="ghost">Back to projects</Button>
  {#if data.error}<Feedback kind="error" title="Project unavailable" message={data.error.message} requestId={data.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Settings not saved" message={form.error.message} requestId={data.requestId} />{/if}
  {#if form?.success}<p role="status">{form.success}</p>{/if}
  {#if data.project}
    <PageHeader eyebrow={data.project.state === "archived" ? "Archived project" : "Project"} title={data.project.displayName} description="Repository readiness, tasks, and retained history." />
    <Card.Root><Card.Header><Card.Title>Environment and readiness</Card.Title></Card.Header><Card.Content class="space-y-3">
      {#each data.profiles as profile (profile.id)}<p><a class="underline" href={`/profiles/${encodeURIComponent(profile.id)}`}>{words(profile.kind)}: {words(profile.state)}</a></p>{:else}<p>Action required: configure a runtime for this repository.</p>{/each}
      {#if data.repository}<Button variant="outline" href={`/repositories/${encodeURIComponent(data.repository.id)}`}>Repository and runtime settings</Button>{/if}
    </Card.Content></Card.Root>
    <Card.Root><Card.Header><Card.Title>Tasks</Card.Title></Card.Header><Card.Content class="space-y-3">
      {#if data.project.state === "active"}<Button href={`/tasks?project=${encodeURIComponent(data.project.id)}#new`}>New task</Button>{:else}<p>New tasks and retries are disabled for this archived project.</p>{/if}
      {#each data.tasks as task (task.id)}<p><a class="underline" href={`/tasks/${encodeURIComponent(task.id)}`}>{task.snapshot.request}</a> · {words(task.state)}</p>{:else}<p>No tasks yet.</p>{/each}
      {#if data.nextPageToken}<Button variant="outline" href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`}>More tasks</Button>{/if}
    </Card.Content></Card.Root>
    <Card.Root><Card.Header><Card.Title>Project settings</Card.Title><Card.Description>{data.repository?.displayName}</Card.Description></Card.Header><Card.Content class="space-y-4">
      {#if data.project.state === "active"}
        <form use:enhance method="POST" action="?/update" class="max-w-xl space-y-3">
          <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} />
          <Label for="name">Project name</Label><Input id="name" name="displayName" value={data.project.displayName} required maxlength={256} /><Button type="submit">Save name</Button>
        </form>
        <form use:enhance method="POST" action="?/archive" class="space-y-3">
          <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} />
          <p class="text-sm text-muted-foreground">Archiving prevents new tasks and retries. Existing work and history are retained.</p><Button type="submit" variant="outline">Archive project</Button>
        </form>
      {/if}
    </Card.Content></Card.Root>
  {/if}
</div>
