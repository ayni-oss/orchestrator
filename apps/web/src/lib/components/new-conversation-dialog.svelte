<script lang="ts">
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { projectHref } from "$lib/project-navigation.js";
  import type { ConversationSidebarData } from "$lib/conversation-sidebar.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  let { data, projectId, open = $bindable(false) }: { data: ConversationSidebarData; projectId?: string | undefined; open?: boolean } = $props();
  const projects = $derived(data.projects.filter(project => project.state === "active" && project.repositoryConnectionIds.length));
  let selected = $state(untrack(() => projects.find(project => project.id === projectId)?.id ?? projects[0]?.id ?? ""));
  let finishClosing: (() => void) | undefined;
  async function startConversation() {
    const href=projectHref(selected,"overview");
    await new Promise<void>(resolve => { finishClosing = resolve; open = false; });
    await goto(`${href}&new=${crypto.randomUUID()}`);
  }
</script>
<Dialog.Root bind:open onOpenChangeComplete={value => { if (!value) { finishClosing?.(); finishClosing = undefined; } }}>
  <Dialog.Content>
    <Dialog.Header><Dialog.Title>New conversation</Dialog.Title><Dialog.Description>Choose a project in {data.workspace.displayName}. Your first message starts the conversation.</Dialog.Description></Dialog.Header>
    <form onsubmit={event=>{event.preventDefault();void startConversation();}} class="space-y-4" data-refresh-paused="">
      <Label for="conversation-project">Project</Label><ThemedSelect id="conversation-project" value={selected} options={projects.map(project => ({ value: project.id, label: project.displayName }))} onValueChange={id => { selected = id; }} required />
      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false}>Cancel</Button><Button type="submit" disabled={!projects.some(project=>project.id===selected)}>Start conversation</Button></Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
