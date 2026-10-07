<script lang="ts">
  import { tick } from "svelte";
  import type { PageProps } from "./$types";
  import { Plus, GitPullRequest, FolderGit2 } from "@lucide/svelte";
  import { superForm } from "sveltekit-superforms";
  import { zodClient } from "sveltekit-superforms/adapters";
  import { projectSchema } from "$lib/project-schema.js";
  import PageHeader from "$lib/components/page-header.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Form from "$lib/components/ui/form/index.js";
  import { formatDate, statusTone } from "$lib/presentation.js";
  let { data }: PageProps = $props();
  let open = $state(false);
  let finishClosing: (() => void) | undefined;
  async function closeBeforeNavigation() {
    if (!open) return;
    // Wait for modal teardown to release focus and pointer locks before replacing the route.
    await new Promise<void>(resolve => {finishClosing=resolve;open=false;});
    await tick();
  }
  const projectForm = (() => superForm(data.form, {validators:zodClient(projectSchema), onError: "apply", onResult:async ({result}) => {if(result.type === "redirect") await closeBeforeNavigation();}}))();
  const { form: values, enhance, submitting, message } = projectForm;
</script>
<svelte:head><title>Projects · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader title="Projects" description="Your repositories, environments, and development tasks.">
    {#snippet actions()}<Button onclick={() => open = true}><Plus aria-hidden="true" /> Create project</Button>{/snippet}
  </PageHeader>
  {#if data.error}<Feedback kind="warning" title="Some project data is unavailable" message={data.error.message} requestId={data.requestId} />{/if}
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each data.projects as project (project.id)}
      {@const repositoryNames = project.repositoryConnectionIds.map(id => data.repositories.find(repo => repo.id === id)?.displayName ?? "Repository unavailable")}
      {@const profiles = data.profiles.filter(profile => project.repositoryConnectionIds.includes(profile.repositoryConnectionId))}
      {@const latest = data.tasksComplete ? data.tasks.filter(task => task.projectId === project.id).sort((a,b) => b.createdAt.localeCompare(a.createdAt))[0] : undefined}
      <Card.Root class="h-full"><Card.Header>
        <Card.Title><a class="hover:underline" href={`/projects/${encodeURIComponent(project.id)}`}>{project.displayName}</a></Card.Title>
        <Card.Description>{repositoryNames.join(", ")}</Card.Description>
      </Card.Header><Card.Content class="space-y-4 text-sm">
        <div class="flex items-center justify-between"><span class="text-muted-foreground">Environment</span><span>{project.state === "archived" ? "Archived" : !data.profilesComplete ? "Unavailable" : project.repositoryConnectionIds.every(id => profiles.some(profile => profile.repositoryConnectionId === id && profile.support === "operational") && profiles.filter(profile => profile.repositoryConnectionId === id && profile.support === "operational").every(profile => profile.state === "ready")) ? "Ready" : "Setup required"}</span></div>
        <div class="flex items-center justify-between gap-3"><span class="text-muted-foreground">Last task</span>{#if latest}<StatusBadge status={latest.state} tone={statusTone(latest.state)} />{:else}<span>{data.tasksComplete ? "No tasks yet" : "Unavailable"}</span>{/if}</div>
        {#if latest}<p class="truncate text-muted-foreground">{latest.snapshot.request} · {formatDate(latest.createdAt)}</p>{/if}
        <div class="flex items-center justify-between"><span class="flex items-center gap-2 text-muted-foreground"><GitPullRequest class="size-4" aria-hidden="true" /> Open PRs</span><span>Unavailable</span></div>
      </Card.Content></Card.Root>
    {:else}<div class="col-span-full rounded-xl border border-dashed p-12 text-center"><FolderGit2 class="mx-auto mb-4 size-8 text-muted-foreground" aria-hidden="true" /><h2 class="font-semibold">Create your first project</h2><p class="mt-2 text-sm text-muted-foreground">Group repositories and start work from one place.</p></div>{/each}
  </div>
  {#if data.nextPageToken}<Button href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`} variant="outline">More projects</Button>{/if}
  <Dialog.Root bind:open onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>Create project</Dialog.Title><Dialog.Description>Choose one or more repositories. Repositories and their runtime setup can be shared across projects.</Dialog.Description></Dialog.Header>
    <form use:enhance method="POST" action="?/create" class="space-y-5">
      <input type="hidden" name="idempotencyKey" value={$values.idempotencyKey} />
      <Form.Field form={projectForm} name="displayName"><Form.Control>{#snippet children({props})}<Form.Label>Project name</Form.Label><Input {...props} bind:value={$values.displayName} autocomplete="off" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={projectForm} name="repositoryConnectionIds"><Form.Control>{#snippet children({props})}<Form.Label>Repositories</Form.Label><RepositoryPicker repositories={data.repositories} bind:value={$values.repositoryConnectionIds} id={props.id} invalid={props["aria-invalid"] === "true"} describedBy={props["aria-describedby"]} />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      {#if !data.repositories.length}<p class="text-sm text-muted-foreground">Connect a repository in Repositories before creating a project.</p><Button href="/repositories" variant="outline">Connect repositories</Button>{/if}
      {#if $message}<p class="text-sm text-destructive" role="alert">{$message}</p>{/if}
      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false}>Cancel</Button><Button type="submit" disabled={$submitting || !data.repositories.length}>{$submitting ? "Creating…" : "Create project"}</Button></Dialog.Footer>
    </form>
  </Dialog.Content></Dialog.Root>
</div>
