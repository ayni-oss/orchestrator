<script lang="ts">
  import { notify, redirectNotification } from "$lib/notifications.js";
  import { tick } from "svelte";
  import type { PageProps } from "./$types";
  import { Plus, GitPullRequest, FolderGit2 } from "@lucide/svelte";
  import { superForm } from "sveltekit-superforms";
  import { zodClient } from "sveltekit-superforms/adapters";
  import { projectSchema } from "$lib/project-schema.js";
  import PageHeader from "$lib/components/page-header.svelte";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
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
  let redirecting = $state(false);
  let finishClosing: (() => void) | undefined;
  async function closeBeforeNavigation() {
    if (!open) return;
    // Wait for modal teardown to release focus and pointer locks before replacing the route.
    await new Promise<void>(resolve => {finishClosing=resolve;open=false;});
    await tick();
  }
  const projectForm = (() => superForm(data.form, {validators:zodClient(projectSchema), onError: ({result}) => notify("error", "Action failed", result.error.message), onResult:async ({result}) => {if(result.type === "redirect") { redirecting = true; await closeBeforeNavigation(); redirectNotification(result, new URL("?/create", window.location.href)); }}}))();
  const { form: values, enhance, submitting } = projectForm;
  const workspace=$derived(data.workspaces.find(item=>item.id===$values.workspaceId));
  const repositories=$derived(data.repositories.filter(item=>workspace?.repositoryConnectionIds.includes(item.id)));

</script>
<svelte:head><title>Projects · Ayni Orchestrator</title></svelte:head>
<div data-refresh-paused={open || $submitting || redirecting ? "" : undefined} class="space-y-8">
  <PageHeader title="Projects" description="Goals, conversations, and development tasks within your workspaces.">
    {#snippet actions()}<Button onclick={() => open = true}><Plus aria-hidden="true" /> Create project</Button>{/snippet}
  </PageHeader>

  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each data.summaries as summary (summary.project.id)}
      {@const project = summary.project}
      {@const latest = summary.tasksComplete ? summary.latestTask : undefined}
      <Card.Root class="relative h-full transition-colors hover:bg-accent/30">
        <a href={`/projects/${encodeURIComponent(project.id)}`} aria-label={project.displayName} class="absolute inset-0 z-10 rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"></a>
        <Card.Header>
        <Card.Title>{project.displayName}</Card.Title>
        <Card.Description>{summary.repositoryNames.join(", ")}</Card.Description>
      </Card.Header><Card.Content class="space-y-4 text-sm">
        <div class="flex items-center justify-between"><span class="text-muted-foreground">Environment</span><span>{project.state === "archived" ? "Archived" : !summary.runtime.complete ? "Unavailable" : summary.runtime.ready ? "Ready" : "Setup required"}</span></div>
        <div class="flex items-center justify-between gap-3"><span class="text-muted-foreground">Last task</span>{#if latest}<StatusBadge status={latest.state} tone={statusTone(latest.state)} />{:else}<span>{summary.tasksComplete ? "No tasks yet" : "Unavailable"}</span>{/if}</div>
        {#if latest}<p class="truncate text-muted-foreground">{latest.request} · {formatDate(latest.createdAt)}</p>{/if}
        <div class="flex items-center justify-between"><span class="flex items-center gap-2 text-muted-foreground"><GitPullRequest class="size-4" aria-hidden="true" /> Open PRs</span><span>Unavailable</span></div>
      </Card.Content></Card.Root>
    {:else}{#if data.available}<div class="col-span-full rounded-xl border border-dashed p-12 text-center"><FolderGit2 class="mx-auto mb-4 size-8 text-muted-foreground" aria-hidden="true" /><h2 class="font-semibold">Create your first project</h2><p class="mt-2 text-sm text-muted-foreground">Group repositories and start work from one place.</p></div>{/if}{/each}
  </div>
  {#if data.nextPageToken}<Button href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`} variant="outline">More projects</Button>{/if}
  <Dialog.Root bind:open onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>Create project</Dialog.Title><Dialog.Description>Choose a workspace, describe your goal, and select repositories within it.</Dialog.Description></Dialog.Header>
    <form use:enhance method="POST" action="?/create" class="space-y-5">
      <input type="hidden" name="idempotencyKey" value={$values.idempotencyKey} />
      <Form.Field form={projectForm} name="workspaceId"><Form.Control>{#snippet children({props})}<Form.Label>Workspace</Form.Label><ThemedSelect {...props} name="workspaceId" value={$values.workspaceId} onValueChange={value=>{$values.workspaceId=value;$values.repositoryConnectionIds=[];}} options={data.workspaces.map(item=>({value:item.id,label:item.displayName}))} placeholder="Choose workspace" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={projectForm} name="goal"><Form.Control>{#snippet children({props})}<Form.Label>Goal</Form.Label><Input {...props} bind:value={$values.goal} placeholder="What should this project achieve?" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={projectForm} name="displayName"><Form.Control>{#snippet children({props})}<Form.Label>Project name</Form.Label><Input {...props} bind:value={$values.displayName} autocomplete="off" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={projectForm} name="repositoryConnectionIds"><Form.Control>{#snippet children({props})}<Form.Label>Repositories</Form.Label><RepositoryPicker repositories={repositories} bind:value={$values.repositoryConnectionIds} id={props.id} invalid={props["aria-invalid"] === "true"} describedBy={props["aria-describedby"]} />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      {#if !repositories.length}<p class="text-sm text-muted-foreground">Choose a workspace with connected repositories before creating a project.</p><Button href="/workspaces" variant="outline">Manage workspaces</Button>{/if}

      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false}>Cancel</Button><Button type="submit" disabled={$submitting || !repositories.length}>{$submitting ? "Creating…" : "Create project"}</Button></Dialog.Footer>
    </form>
  </Dialog.Content></Dialog.Root>
</div>
