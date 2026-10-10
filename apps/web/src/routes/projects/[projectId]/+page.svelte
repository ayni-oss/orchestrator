<script lang="ts">
  import { enhance } from "$lib/forms.js";
  import type { PageProps } from "./$types";
  import AyniIcon from "$lib/components/ayni-icon.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import NewTaskForm from "$lib/components/new-task-form.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { formatDate, statusTone } from "$lib/presentation.js";
  let { data, form }: PageProps = $props();
  let finishClosing: (()=>void) | undefined;
  async function closeTask() {
    if (!newTask) return;
    await new Promise<void>(resolve=>{finishClosing=resolve;newTask=false;});
  }
  let newTask=$state(false), members=$state<string[]>([]), editing=$state(false);
  const selectedMembers=$derived(editing ? members : data.project?.repositoryConnectionIds ?? []);
</script>
<svelte:head><title>{data.project?.displayName ?? "Project unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">

  {#if data.project}
    <PageHeader eyebrow={data.project.state === "archived" ? "Archived project" : "Project"} title={data.project.displayName} description="Repositories, environments, and task history.">{#snippet actions()}{#if data.project?.state === "active"}<Button onclick={() => newTask=true}>New task</Button>{/if}{/snippet}</PageHeader>
    <section class="space-y-4">
      <h2 class="text-lg font-semibold">Repositories</h2>
      <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {#each data.repositories as repository (repository.id)}
          <Card.Root>
            <Card.Content class="flex items-center justify-between gap-3">
              <a href={`/repositories/${encodeURIComponent(repository.id)}`} class="min-w-0 break-all font-semibold hover:underline">{repository.displayName}</a>
              {#if repository.useAyni !== false}
                <AyniIcon class="size-5 shrink-0 text-muted-foreground" role="img" aria-label="Ayni enabled" />
              {/if}
            </Card.Content>
          </Card.Root>
        {/each}
      </div>
    </section>
    <section class="space-y-4"><h2 class="text-lg font-semibold">Tasks</h2>{#if data.project.state === "archived"}<p>New tasks and retries are disabled for this archived project.</p>{/if}<div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      {#each data.tasks as task (task.id)}<Card.Root><Card.Header><Card.Title class="text-base"><a href={`/tasks/${encodeURIComponent(task.id)}`} class="hover:underline">{task.snapshot.request}</a></Card.Title><Card.Description>{data.repositories.find(repo=>repo.id===task.snapshot.repository.repositoryConnectionId)?.displayName ?? "Removed repository · history retained"}</Card.Description></Card.Header><Card.Content class="space-y-3 text-sm"><StatusBadge status={task.state} tone={statusTone(task.state)} /><p class="text-muted-foreground">Created {formatDate(task.createdAt)}</p><p class="text-muted-foreground">Usage and PR results available in task details</p></Card.Content></Card.Root>{:else}<p class="text-sm text-muted-foreground">No tasks yet.</p>{/each}
    </div>{#if data.nextPageToken}<Button variant="outline" href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`}>More tasks</Button>{/if}</section>
    {#if data.project.state === "active"}<Card.Root><Card.Header><Card.Title>Project settings</Card.Title><Card.Description>Membership changes retain task history. Removing a repository prevents new tasks and retries for it in this project.</Card.Description></Card.Header><Card.Content class="space-y-6">
      <form use:enhance={() => { return async ({update}) => {await update();editing=false;};}} method="POST" action="?/update" class="max-w-xl space-y-3">
        <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} />
        <Label for="name">Project name</Label><Input id="name" name="displayName" value={data.project.displayName} required maxlength={256} />
        <Label for="members">Repositories</Label><RepositoryPicker id="members" repositories={data.choices} value={selectedMembers} onchange={value => {members=value;editing=true;}} />
        <Button type="submit">Save settings</Button>
      </form>
      <form use:enhance method="POST" action="?/archive" class="space-y-3"><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><input type="hidden" name="resourceVersion" value={data.project.resourceVersion} /><p class="text-sm text-muted-foreground">Archiving prevents new tasks and retries. Existing work and history are retained.</p><Button type="submit" variant="outline">Archive project</Button></form>
    </Card.Content></Card.Root>{/if}
    <Dialog.Root bind:open={newTask} onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>New task</Dialog.Title><Dialog.Description>Choose a repository and describe the work for {data.project.displayName}.</Dialog.Description></Dialog.Header><NewTaskForm project={data.project} repositories={data.repositories} profiles={data.profiles} models={data.models} provider={data.provider} idempotencyKey={data.taskKey} onSubmitted={closeTask} /></Dialog.Content></Dialog.Root>
  {/if}
</div>
