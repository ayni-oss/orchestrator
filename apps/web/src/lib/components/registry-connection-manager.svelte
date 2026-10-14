<script lang="ts">
  import { onMount } from "svelte";
  import { enhance } from "$lib/forms.js";
  import { refreshScope, subscribeRefresh } from "$lib/refresh.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { ConnectionView } from "$lib/connection.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { statusTone } from "$lib/presentation.js";

  let { connections, idempotencyKey, available = true }: {
    connections: ConnectionView[];
    idempotencyKey: string;
    available?: boolean;
  } = $props();
  let createOpen = $state(false);
  let editOpen = $state(false);
  let selectedId = $state("");
  const selected = $derived(connections.find(connection => connection.id === selectedId));
  let createPush = $state("");
  let createPull = $state("");
  let editPush = $state("");
  let editPull = $state("");
  let submitting = $state(false);
  onMount(() => subscribeRefresh("app:connections", {intervalMs:30_000}));
  function closeCreate(open: boolean) {
    createOpen = open;
    if (!open) { createPush = ""; createPull = ""; }
  }
  function closeEdit(open: boolean) {
    editOpen = open;
    if (!open) { editPush = ""; editPull = ""; }
  }
  function openEdit(id: string) {
    selectedId = id;
    editPush = "";
    editPull = "";
    editOpen = true;
  }
  function submit(operation: string): SubmitFunction {
    return () => {
      submitting = true;
      return async ({ result, update }) => {
        createPush = ""; createPull = ""; editPush = ""; editPull = "";
        try {
          await update({invalidateAll:false});
          if (result.type === "success" && operation === "create") closeCreate(false);
          if (result.type === "success" && operation === "delete") closeEdit(false);
          await refreshScope("app:connections", true);
        } finally { submitting = false; }
      };
    };
  }
</script>

{#snippet permissions()}
  <p class="text-sm text-muted-foreground">Use separate classic personal access tokens. The pull token requires read:packages only; write:packages and delete:packages are rejected. The push token requires write:packages; add delete:packages to clean up test artifacts. Both tokens need access to the selected private namespace. Ayni does not grant GitHub permissions.</p>
{/snippet}

<div class="space-y-4">
  <div class="flex items-center justify-between gap-3">
    <h2 class="text-lg font-semibold">Connections</h2>
    <Button onclick={() => { createOpen = true; }} disabled={!available}>Add connection</Button>
  </div>
  {#if !available}<p role="status">Connection service unavailable. Retry when the control plane is ready.</p>{/if}
  {#if !connections.length}<p class="text-sm text-muted-foreground">Add a connection to store images for repository testing environments.</p>{/if}
  {#each connections as connection (connection.id)}
    {#snippet cleanupStatus()}<StatusBadge status="cleanup_required" tone="warning" />{/snippet}
    <InformationCard title={connection.displayName} status={connection.enabled ? connection.status : "disabled"} actions={[{ label: connection.managementSource === "admin" ? "Details" : "Edit", onSelect: () => openEdit(connection.id) }]}>
      <InformationTable rows={[
        { label: "Image namespace", value: `ghcr.io/${connection.imageNamespace}` },
        ...(connection.testCleanup === "failed" ? [{ label: "Test cleanup", value: cleanupStatus }] : []),
        ...(connection.managementSource === "admin" ? [{ label: "Managed by", value: "Your administrator" }] : []),
      ]} />
      {#if connection.failureReason}<Feedback kind="warning" title="Latest operation needs attention" message={connection.failureReason} />{/if}
    </InformationCard>
  {/each}
</div>

<Dialog.Root open={createOpen} onOpenChange={closeCreate}>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header>
      <Dialog.Title>Add GHCR connection</Dialog.Title>
      <Dialog.Description>Provide separate push and pull credentials for your image namespace.</Dialog.Description>
    </Dialog.Header>

    <form method="POST" action="?/create" use:enhance={submit("create")} class="space-y-4">
      <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
      <div class="space-y-2"><Label for="registry-name">Connection name</Label><Input id="registry-name" name="displayName" required maxlength={128} /></div>
      <div class="grid gap-4 sm:grid-cols-2">
        <div class="space-y-2"><Label for="registry-username">GitHub username</Label><Input id="registry-username" name="username" required /></div>
        <div class="space-y-2"><Label for="registry-namespace">Image namespace</Label><Input id="registry-namespace" name="imageNamespace" required placeholder="organization" /></div>
      </div>
      <p class="text-sm text-muted-foreground">Set the image namespace to the repository owner to publish at ghcr.io/&lt;owner&gt;/&lt;repository&gt;. The GitHub username is used only for authentication.</p>
      <div class="space-y-2"><Label for="registry-push">Push token</Label><Input id="registry-push" name="token" type="password" required autocomplete="new-password" bind:value={createPush} /></div>
      <div class="space-y-2"><Label for="registry-pull">Separate pull-only token</Label><Input id="registry-pull" name="pullToken" type="password" required autocomplete="new-password" bind:value={createPull} /></div>
      {@render permissions()}
      <p class="text-sm text-muted-foreground">Credentials are stored in Kubernetes Secrets and are never returned by this page. Validated credentials activate automatically.</p>
      <div class="flex justify-end gap-2"><Button variant="outline" onclick={() => closeCreate(false)}>Cancel</Button><Button type="submit" disabled={submitting}>Save and test</Button></div>
    </form>
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root open={editOpen} onOpenChange={closeEdit}>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header>
      <Dialog.Title>{selected?.managementSource === "admin" ? "Connection details" : "Edit connection"}</Dialog.Title>
      <Dialog.Description>{selected ? `${selected.displayName} · ghcr.io/${selected.imageNamespace}` : "This connection is no longer available."}</Dialog.Description>
    </Dialog.Header>

    {#if selected}
      <div class="flex flex-wrap gap-2"><StatusBadge status={selected.enabled ? selected.status : "disabled"} tone={statusTone(selected.enabled ? selected.status : "disabled")} />{#if selected.testCleanup === "failed"}<StatusBadge status="cleanup_required" tone="warning" />{/if}</div>
      <dl class="grid grid-cols-2 gap-2 text-sm">
        {#each [["Authentication", selected.testAuthentication], ["Push", selected.testPush], ["Pull", selected.testPull], ["Cleanup", selected.testCleanup]] as [label, status] (label)}
          <dt class="text-muted-foreground">{label}</dt><dd>{status || "Not tested"}</dd>
        {/each}
      </dl>
      {#if selected.failureReason}<Feedback kind="warning" title="Connection needs attention" message={selected.failureReason} />{/if}
      {#if selected.testCleanupFailureReason}<Feedback kind="warning" title="Artifact cleanup required" message={`${selected.testCleanupFailureReason}. Retry after restoring package deletion permission.`} />{/if}
      {#if selected.managementSource === "admin"}
        <p>Managed by your administrator. Update credentials in your deployment’s secret store.</p>
      {:else}
        <form method="POST" action="?/replace" use:enhance={submit("replace")} class="space-y-4">
          <input type="hidden" name="id" value={selected.id} /><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-replace-${selected.id}`} /><input type="hidden" name="resourceVersion" value={selected.resourceVersion} />
          <div class="space-y-2"><Label for="registry-edit-push">New push token</Label><Input id="registry-edit-push" name="token" type="password" required autocomplete="new-password" bind:value={editPush} /></div>
          <div class="space-y-2"><Label for="registry-edit-pull">Separate pull-only token</Label><Input id="registry-edit-pull" name="pullToken" type="password" required autocomplete="new-password" bind:value={editPull} /></div>
          {@render permissions()}
          <Button type="submit" disabled={submitting}>Replace credentials</Button>
        </form>
        <div class="flex flex-wrap gap-2 border-t pt-4">
          {#each ["test", "toggle", "delete"] as action (action)}
            <form method="POST" action={`?/${action}`} use:enhance={submit(action)}>
              <input type="hidden" name="id" value={selected.id} /><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-${action}-${selected.id}`} /><input type="hidden" name="resourceVersion" value={selected.resourceVersion} /><input type="hidden" name="enabled" value={String(!selected.enabled)} />
              <Button type="submit" variant={action === "delete" ? "destructive" : "outline"} disabled={submitting}>{action === "test" ? "Test connection / Retry" : action === "toggle" ? selected.enabled ? "Disable" : "Enable" : "Delete"}</Button>
            </form>
          {/each}
        </div>
      {/if}
    {/if}
  </Dialog.Content>
</Dialog.Root>
