<script lang="ts">
  import type { PageData } from "../../routes/_old/workspaces/[workspaceId]/$types";
  import { enhance } from "$lib/forms.js";
  import Feedback from "$lib/components/feedback.svelte";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";

  let { data }: { data: PageData } = $props();
  let open = $state(false), saving = $state(false), error = $state("");
  let members = $state<string[]>([]);
  function manage() {
    members = [...(data.workspace?.repositoryConnectionIds ?? [])];
    error = "";
    open = true;
  }
</script>

<div data-refresh-paused={open || saving ? "" : undefined}>
  <Button onclick={manage}>Manage repositories</Button>
  <Dialog.Root bind:open>
    <Dialog.Content class="sm:max-w-xl">
      <Dialog.Header>
        <Dialog.Title>Manage repositories</Dialog.Title>
        <Dialog.Description>Choose existing repositories for this workspace. Removing a repository prevents new work using it here and preserves existing history.</Dialog.Description>
      </Dialog.Header>
      {#if data.workspace}
        <form method="POST" action="?/update" class="space-y-4" use:enhance={()=>{
          saving=true;error="";
          return async({result,update})=>{
            try {
              if(result.type==="failure")error=String(result.data?.error?.message??"Repository membership could not be saved. Try again.");
              await update({reset:false});
              if(result.type==="success")open=false;
            } finally { saving=false; }
          };
        }}>
          <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
          <input type="hidden" name="resourceVersion" value={data.workspace.resourceVersion} />
          <input type="hidden" name="displayName" value={data.workspace.displayName} />
          <Label for="workspace-repositories">Add or remove existing repositories</Label>
          <RepositoryPicker id="workspace-repositories" repositories={data.repositories} bind:value={members} />
          {#if error}<Feedback kind="error" title="Repository membership not saved" message={error} />{/if}
          <Dialog.Footer>
            <Button type="button" variant="outline" onclick={()=>open=false} disabled={saving}>Cancel</Button>
            <Button type="submit" disabled={saving}>{saving?"Saving…":"Save repository membership"}</Button>
          </Dialog.Footer>
        </form>
      {/if}
    </Dialog.Content>
  </Dialog.Root>
</div>
