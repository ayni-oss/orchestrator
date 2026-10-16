<script lang="ts">
  import { notify, redirectNotification } from "$lib/notifications.js";
  import { tick } from "svelte";
  import type { PageProps } from "./$types";
  import { Plus } from "@lucide/svelte";
  import { superForm } from "sveltekit-superforms";
  import { zodClient } from "sveltekit-superforms/adapters";
  import { workspaceSchema } from "$lib/workspace-schema.js";
  import PageHeader from "$lib/components/page-header.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Form from "$lib/components/ui/form/index.js";
  import { statusTone } from "$lib/presentation.js";
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
  const workspaceForm = (() => superForm(data.form, {validators:zodClient(workspaceSchema), onError: ({result}) => notify("error", "Action failed", result.error.message), onResult:async ({result}) => {if(result.type === "redirect") { redirecting = true; await closeBeforeNavigation(); redirectNotification(result, new URL("?/create", window.location.href)); }}}))();
  const { form: values, enhance, submitting } = workspaceForm;
</script>
<svelte:head><title>Workspaces · Ayni Orchestrator</title></svelte:head>
<div data-refresh-paused={$submitting || redirecting ? "" : undefined} class="space-y-8">
  <PageHeader title="Workspaces" description="Your projects, available repositories, and shared agent settings.">
    {#snippet actions()}<Button onclick={() => open = true}><Plus aria-hidden="true" /> Create workspace</Button>{/snippet}
  </PageHeader>

  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each data.workspaces as workspace (workspace.id)}
      <Card.Root><Card.Header><Card.Title><a class="hover:underline" href={`/workspaces/${encodeURIComponent(workspace.id)}`}>{workspace.displayName}</a></Card.Title><Card.Description>{data.projectCounts ? `${data.projectCounts[workspace.id]??0} ${(data.projectCounts[workspace.id]??0)===1?"project":"projects"}` : "Project count unavailable"} · {workspace.repositoryConnectionIds.length} {workspace.repositoryConnectionIds.length===1?"repository":"repositories"}</Card.Description></Card.Header><Card.Content><StatusBadge status={workspace.state} tone={statusTone(workspace.state)} /></Card.Content></Card.Root>
    {:else}{#if data.available}<p class="text-sm text-muted-foreground">Create a workspace to share repositories and agent defaults across projects.</p>{/if}{/each}
  </div>
  {#if data.nextPageToken}<Button href={`?pageToken=${encodeURIComponent(data.nextPageToken)}`} variant="outline">More workspaces</Button>{/if}
  <Dialog.Root bind:open onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>Create workspace</Dialog.Title><Dialog.Description>Select repositories to make available to projects in this workspace.</Dialog.Description></Dialog.Header>
    <form use:enhance method="POST" action="?/create" class="space-y-5">
      <input type="hidden" name="idempotencyKey" value={$values.idempotencyKey} />
      <Form.Field form={workspaceForm} name="displayName"><Form.Control>{#snippet children({props})}<Form.Label>Workspace name</Form.Label><Input {...props} bind:value={$values.displayName} autocomplete="off" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={workspaceForm} name="repositoryConnectionIds"><Form.Control>{#snippet children({props})}<Form.Label>Repositories</Form.Label><RepositoryPicker repositories={data.repositories} bind:value={$values.repositoryConnectionIds} id={props.id} invalid={props["aria-invalid"] === "true"} describedBy={props["aria-describedby"]} />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      {#if !data.repositories.length}<p class="text-sm text-muted-foreground">Connect repositories before creating a workspace.</p><Button href="/repositories" variant="outline">Connect repositories</Button>{/if}

      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false}>Cancel</Button><Button type="submit" disabled={$submitting || !data.repositories.length}>{$submitting ? "Creating…" : "Create workspace"}</Button></Dialog.Footer>
    </form>
  </Dialog.Content></Dialog.Root>
</div>
