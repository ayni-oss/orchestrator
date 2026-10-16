<script lang="ts">
  import { untrack } from "svelte";
  import type { ConversationSidebarData } from "$lib/conversation-sidebar.js";
  import { enhance } from "$lib/forms.js";
  import RepositoryPicker from "$lib/components/repository-picker.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  let { data, idempotencyKey, projectId, open = $bindable(false) }: { data: ConversationSidebarData; idempotencyKey: string; projectId?: string | undefined; open?: boolean } = $props();
  const projects = $derived(data.projects.filter(project => project.state === "active" && project.repositoryConnectionIds.length));
  let selected = $state(untrack(() => projects.find(project => project.id === projectId)?.id ?? projects[0]?.id ?? ""));
  const project = $derived(projects.find(project => project.id === selected));
  let members = $state(untrack(() => [...(project?.repositoryConnectionIds ?? [])]));
  let pending = $state(false), error = $state("");
  let finishClosing: (() => void) | undefined;
  async function closeBeforeNavigation() { await new Promise<void>(resolve => { finishClosing = resolve; open = false; }); }
</script>
<Dialog.Root bind:open onOpenChangeComplete={value => { if (!value) { finishClosing?.(); finishClosing = undefined; } }}>
  <Dialog.Content>
    <Dialog.Header><Dialog.Title>New conversation</Dialog.Title><Dialog.Description>Choose a project and repositories in {data.workspace.displayName} to research.</Dialog.Description></Dialog.Header>
    <form method="POST" action={`/projects/${encodeURIComponent(selected)}?/createConversation`} use:enhance={() => { pending = true; error = ""; return async ({ result, update }) => { try { if (result.type === "failure") error = String(result.data?.error?.message ?? "Conversation could not be created. Try again."); if (result.type === "redirect") await closeBeforeNavigation(); await update({ reset: false }); } finally { pending = false; } }; }} class="space-y-4" data-refresh-paused="">
      <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
      <Label for="conversation-project">Project</Label><ThemedSelect id="conversation-project" value={selected} options={projects.map(project => ({ value: project.id, label: project.displayName }))} onValueChange={id => { selected = id; members = [...(projects.find(project => project.id === id)?.repositoryConnectionIds ?? [])]; }} required disabled={pending} />
      <Label for="conversation-title">Title</Label><Input id="conversation-title" name="title" required maxlength={256} disabled={pending} />
      <Label for="conversation-repositories">Repositories</Label><RepositoryPicker id="conversation-repositories" repositories={data.repositories.filter(repository => project?.repositoryConnectionIds.includes(repository.id))} bind:value={members} />
      {#if error}<Feedback kind="error" title="Conversation not created" message={error} />{/if}
      <Dialog.Footer><Button type="button" variant="outline" onclick={() => open = false} disabled={pending}>Cancel</Button><Button type="submit" disabled={pending || !members.length || !project}>{pending ? "Creating…" : "Create conversation"}</Button></Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
