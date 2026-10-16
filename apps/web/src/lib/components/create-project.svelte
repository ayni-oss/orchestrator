<script lang="ts">
  import { notify, redirectNotification } from "$lib/notifications.js";
  import { tick } from "svelte";
  import type { PageData } from "../../routes/workspaces/[workspaceId]/$types";
  import { Plus } from "@lucide/svelte";
  import { superForm } from "sveltekit-superforms";
  import { zodClient } from "sveltekit-superforms/adapters";
  import { projectSchema } from "$lib/project-schema.js";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Form from "$lib/components/ui/form/index.js";
  let { data }: { data: PageData } = $props();
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
  const workspace=$derived(data.workspace);
  const repositories=$derived(data.repositories.filter(item=>workspace?.repositoryConnectionIds.includes(item.id)));

</script>
<div data-refresh-paused={open || $submitting || redirecting ? "" : undefined}>
<Button onclick={() => open = true}><Plus aria-hidden="true" /> Create project</Button>
  <Dialog.Root bind:open onOpenChangeComplete={value=>{if(!value){finishClosing?.();finishClosing=undefined;}}}><Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>Create project</Dialog.Title><Dialog.Description>Describe your goal and select repositories available in this workspace.</Dialog.Description></Dialog.Header>
    <form use:enhance method="POST" action="?/create" class="space-y-5">
      <input type="hidden" name="idempotencyKey" value={$values.idempotencyKey} />
      <input type="hidden" name="workspaceId" value={data.workspace?.id} />
      <p class="text-sm text-muted-foreground">Workspace: {data.workspace?.displayName}</p>
      <Form.Field form={projectForm} name="goal"><Form.Control>{#snippet children({props})}<Form.Label>Goal</Form.Label><Input {...props} bind:value={$values.goal} placeholder="What should this project achieve?" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={projectForm} name="displayName"><Form.Control>{#snippet children({props})}<Form.Label>Project name</Form.Label><Input {...props} bind:value={$values.displayName} autocomplete="off" />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      <Form.Field form={projectForm} name="repositoryConnectionIds"><Form.Control>{#snippet children({props})}<Form.Label>Repositories</Form.Label><RepositoryPicker repositories={repositories} bind:value={$values.repositoryConnectionIds} id={props.id} invalid={props["aria-invalid"] === "true"} describedBy={props["aria-describedby"]} />{/snippet}</Form.Control><Form.FieldErrors /></Form.Field>
      {#if !repositories.length}<p class="text-sm text-muted-foreground">Choose a workspace with connected repositories before creating a project.</p><Button href="/workspaces" variant="outline">Manage workspaces</Button>{/if}

      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false}>Cancel</Button><Button type="submit" disabled={$submitting || !repositories.length}>{$submitting ? "Creating…" : "Create project"}</Button></Dialog.Footer>
    </form>
  </Dialog.Content></Dialog.Root>
</div>
