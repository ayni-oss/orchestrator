<script lang="ts">
  import { onMount } from "svelte";
  import { enhance } from "$app/forms";
  import { invalidateAll } from "$app/navigation";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { ConnectionView } from "$lib/connection.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import { statusTone } from "$lib/presentation.js";
  let { connections, idempotencyKey, available = true, error }: { connections: ConnectionView[]; idempotencyKey: string; available?: boolean; error?: string | undefined } = $props();
  let createOpen = $state(false);
  let editOpen = $state(false);
  let selectedId = $state("");
  const selected = $derived(connections.find(connection => connection.id === selectedId));
  let mode = $state<"create" | "import">("create");
  let privateKey = $state("");
  let replacementKey = $state("");
  let submitting = $state(false);
  let showError = $state(false);
  let manifestError = $state("");
  onMount(() => { const timer = setInterval(() => { void invalidateAll(); }, 30_000); return () => clearInterval(timer); });
  function closeCreate(open: boolean) { createOpen = open; if (!open) { privateKey = ""; showError = false; } }
  function closeEdit(open: boolean) { editOpen = open; if (!open) { replacementKey = ""; showError = false; } }
  function openCreate() { createOpen = true; mode = "create"; showError = false; manifestError = ""; void prepareManifest(); }
  function openEdit(id: string) { selectedId = id; replacementKey = ""; showError = false; editOpen = true; }
  function submit(operation: string): SubmitFunction {
    return () => { submitting = true; showError = false; return async ({result, update}) => {
      privateKey = ""; replacementKey = "";
      try { await update(); showError = result.type === "failure" || result.type === "error";
        if (result.type === "success" && operation === "create") closeCreate(false);
        if (result.type === "success" && operation === "delete") closeEdit(false);
        await invalidateAll();
      } finally { submitting = false; }
    }; };
  }
  let owner = $state("");
  const ownerIsValid = $derived(/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,98}[A-Za-z0-9])?$/u.test(owner));
  let flowState = $state("");
  let callbackOrigin = $state("");
  let appSuffix = $state("");
  const organizationAppUrl = $derived(ownerIsValid && flowState ? `https://github.com/organizations/${encodeURIComponent(owner)}/settings/apps/new?state=${encodeURIComponent(flowState)}` : undefined);
  const personalAppUrl = $derived(flowState ? `https://github.com/settings/apps/new?state=${encodeURIComponent(flowState)}` : undefined);
  const appName = $derived(`Ayni Orchestrator (${appSuffix})`);
  const appManifest = $derived(JSON.stringify({
    name: appName,
    description: "Customer-owned GitHub App for a trusted local Ayni Orchestrator deployment.",
    url: "https://github.com/ayni-oss/orchestrator",
    redirect_url: callbackOrigin ? `${callbackOrigin}/source-control/github/complete` : undefined,
    public: false,
    default_permissions: {
      contents: "write",
      metadata: "read",
      pull_requests: "write",
      workflows: "write",
    },
  }));
  async function prepareManifest() {
    callbackOrigin = window.location.origin;
    appSuffix = Array.from(crypto.getRandomValues(new Uint8Array(4)), byte => byte.toString(16).padStart(2, "0")).join("");
    flowState = "";
    try {
      const response = await fetch("/source-control/github/start", { method: "POST" });
      if (response.ok) flowState = (await response.json() as { state: string }).state;
      else manifestError = "GitHub setup could not be prepared. Close this dialog and try again.";
    } catch { manifestError = "GitHub setup could not be prepared. Close this dialog and try again."; }
  }
</script>
{#snippet guidance()}
  <p class="text-sm text-muted-foreground">Repository permissions: Metadata read-only; Contents, Pull requests, and Workflows read and write. Install the App only on repositories Ayni should access. App ownership and installation permissions stay with your GitHub account or organization.</p>
{/snippet}
<div class="space-y-4">
  <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">GitHub App</h2>{#if !connections.length}<Button onclick={openCreate} disabled={!available}>Add GitHub App</Button>{/if}</div>
  {#if !available}<p role="status">Connection service unavailable. Retry when the control plane is ready.</p>{/if}
  {#if !connections.length}<p class="text-sm text-muted-foreground">Add a GitHub App to connect your repositories.</p>{/if}
  {#each connections as connection (connection.id)}
    <section aria-label={connection.displayName} class="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-card px-4 py-3">
      <div class="min-w-0 flex-1"><h3 class="truncate font-medium">{connection.displayName}</h3><p class="text-sm text-muted-foreground">App ID: {connection.appId}</p>{#if connection.managementSource === "admin"}<p class="text-xs text-muted-foreground">Managed by your administrator</p>{/if}</div>
      <div class="flex items-center gap-2"><StatusBadge status={connection.enabled ? connection.status : "disabled"} tone={statusTone(connection.enabled ? connection.status : "disabled")} /><Button variant="outline" size="sm" onclick={() => openEdit(connection.id)}>{connection.managementSource === "admin" ? "Details" : "Edit"}</Button></div>
    </section>
  {/each}
</div>
<Dialog.Root open={createOpen} onOpenChange={closeCreate}>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header><Dialog.Title>Add GitHub App</Dialog.Title><Dialog.Description>Create an App in GitHub or import an existing App.</Dialog.Description></Dialog.Header>
    <div class="flex gap-2"><Button variant={mode === "create" ? "default" : "outline"} onclick={() => { mode = "create"; privateKey = ""; }}>Create App</Button><Button variant={mode === "import" ? "default" : "outline"} onclick={() => { mode = "import"; }}>Import existing App</Button></div>
    {#if showError && error}<Feedback kind="error" title="Connection failed" message={error} />{/if}
    {#if mode === "create"}
      {@render guidance()}
      <p class="text-sm text-muted-foreground">GitHub creates the private key through its App Manifest flow. Ayni stores the generated credentials securely and returns you to the connection status page.</p>
      {#if manifestError}<Feedback kind="error" title="Setup unavailable" message={manifestError} />{/if}
      <div class="space-y-2"><Label for="github-owner">GitHub organization</Label><Input id="github-owner" bind:value={owner} autocomplete="organization" placeholder="acme-inc" /></div>
      <p class="text-sm text-muted-foreground">App name: {appName}</p>
      {#if organizationAppUrl}<form action={organizationAppUrl} method="POST" target="_blank"><input type="hidden" name="manifest" value={appManifest} /><Button type="submit">Create App for {owner}</Button></form>{:else}<Button disabled>{flowState ? "Enter an organization name to continue" : "Preparing GitHub setup…"}</Button>{/if}
      {#if personalAppUrl}<form action={personalAppUrl} method="POST" target="_blank"><input type="hidden" name="manifest" value={appManifest} /><Button type="submit" variant="outline">Create App for personal account</Button></form>{/if}
      <div class="flex justify-end"><Button variant="outline" onclick={() => closeCreate(false)}>Cancel</Button></div>
    {:else}
      <form method="POST" action="?/create" use:enhance={submit("create")} class="space-y-4">
        <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
        <div class="space-y-2"><Label for="github-name">Connection name</Label><Input id="github-name" name="displayName" required maxlength={128} /></div>
        <div class="space-y-2"><Label for="github-id">App ID</Label><Input id="github-id" name="appId" required inputmode="numeric" /></div>
        <div class="space-y-2"><Label for="github-key">Private key (PEM)</Label><textarea id="github-key" name="privateKey" required autocomplete="off" bind:value={privateKey} class="min-h-28 w-full rounded-md border bg-background p-3 text-sm"></textarea></div>
        {@render guidance()}
        <p class="text-sm text-muted-foreground">Credentials are stored in Kubernetes Secrets and are never returned by this page. Validated credentials activate automatically.</p>
        <div class="flex justify-end gap-2"><Button variant="outline" onclick={() => closeCreate(false)}>Cancel</Button><Button type="submit" disabled={submitting}>Save and test</Button></div>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
<Dialog.Root open={editOpen} onOpenChange={closeEdit}>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header><Dialog.Title>{selected?.managementSource === "admin" ? "App details" : "Edit GitHub App"}</Dialog.Title><Dialog.Description>{selected ? `${selected.displayName} · App ID: ${selected.appId}` : "This connection is no longer available."}</Dialog.Description></Dialog.Header>
    {#if showError && error}<Feedback kind="error" title="Connection update failed" message={error} />{/if}
    {#if selected}
      <StatusBadge status={selected.enabled ? selected.status : "disabled"} tone={statusTone(selected.enabled ? selected.status : "disabled")} />
      {#if selected.failureReason}<Feedback kind="warning" title="Connection needs attention" message={selected.failureReason} />{/if}
      {#if selected.managementSource === "admin"}<p>Managed by your administrator. Update credentials in your deployment’s secret store.</p>
      {:else}
        <form method="POST" action="?/replace" use:enhance={submit("replace")} class="space-y-4">
          <input type="hidden" name="id" value={selected.id} /><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-replace-${selected.id}`} /><input type="hidden" name="resourceVersion" value={selected.resourceVersion} />
          <div class="space-y-2"><Label for="github-replacement">New private key</Label><textarea id="github-replacement" name="privateKey" required autocomplete="off" bind:value={replacementKey} class="min-h-28 w-full rounded-md border bg-background p-3 text-sm"></textarea></div>
          <p class="text-sm text-muted-foreground">Generate a new private key for this same App in GitHub. The current key remains active until its replacement passes validation.</p><Button type="submit" disabled={submitting}>Replace credentials</Button>
        </form>
        <div class="flex flex-wrap gap-2 border-t pt-4">{#each ["test", "toggle", "delete"] as action (action)}
          <form method="POST" action={`?/${action}`} use:enhance={submit(action)}><input type="hidden" name="id" value={selected.id} /><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-${action}-${selected.id}`} /><input type="hidden" name="resourceVersion" value={selected.resourceVersion} /><input type="hidden" name="enabled" value={String(!selected.enabled)} /><Button type="submit" variant={action === "delete" ? "destructive" : "outline"} disabled={submitting}>{action === "test" ? "Test connection / Retry" : action === "toggle" ? selected.enabled ? "Disable" : "Enable" : "Delete"}</Button></form>
        {/each}</div>
      {/if}
    {/if}
  </Dialog.Content>
</Dialog.Root>
