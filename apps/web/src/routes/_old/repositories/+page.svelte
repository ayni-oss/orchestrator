<script lang="ts">
  import { registryImageRepository } from "$lib/connection.js";
  import { notify } from "$lib/notifications.js";
  import type { RepositoryOptions } from "$lib/server/repository-options.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { enhance } from "$lib/forms.js";
  import { Plus } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import RepositoryCard from "$lib/components/repository-card.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Checkbox from "$lib/components/ui/checkbox/index.js";
  import { Label } from "$lib/components/ui/label/index.js";

  let { data }: PageProps = $props();
  let connecting = $state(false);
  let connectOpen = $state(false);
  let options = $state<RepositoryOptions>({ installations: [], selectedAppName: "", selectedInstallationId: "", repositories: [] });
  let loadingOptions = $state(false);
  let optionsRequest = 0;
  async function loadOptions(appName = "", installationId = "") {
    const sequence = ++optionsRequest;
    loadingOptions = true;
    selectedRepository = "";
    delete options.error;
    try {
      const query = new URLSearchParams({ appName, installationId });
      const response = await fetch(`/_old/repositories/options?${query}`);
      const result = await response.json();
      if (sequence !== optionsRequest) return;
      if (response.ok) options = result;
      else { options.error = result.error; notify("error", "GitHub unavailable", options.error?.message, options.error?.requestId); }
    } catch {
      if (sequence === optionsRequest) { options.error = { status: 503, reason: "provider_unavailable", message: "GitHub options could not be loaded. Try again.", requestId: data.requestId, retryable: true }; notify("error", "GitHub unavailable", options.error.message, options.error.requestId); }
    } finally {
      if (sequence === optionsRequest) loadingOptions = false;
    }
  }
  function openConnect() {
    useAyni = false;
    selectedRegistryId = "";
    connectOpen = true;
    void loadOptions();
  }
  let selectedRepository = $state("");
  let selectedRegistryId = $state("");
  let useAyni = $state(false);
  const selectedRegistry = $derived(data.registryConnections.find(connection => connection.id === selectedRegistryId));
  const selectedOwner = $derived(options.installations.find(item => item.installationId === options.selectedInstallationId)?.owner ?? "");
  const imageDestination = $derived(selectedRegistry ? registryImageRepository(selectedOwner, selectedRepository) : "");
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/_old/repositories?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Repository connections · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Connectivity" title="Repository connections" description="Connect repositories to make them available to Orchestrator. It builds runtimes from their Ayni configuration; for repositories without Ayni, provide your own container image.">
    {#snippet actions()}<Button onclick={openConnect}><Plus aria-hidden="true" /> Connect repository</Button>{/snippet}
  </PageHeader>

  <section class="space-y-4">
    {#if data.summaries.length === 0 && data.backend === "connected"}
      <EmptyState title="No repository connections" description="Connect a repository to discover its Ayni configuration, or provide your own container image for its runtime." />
    {:else}
      <div class="grid gap-4 lg:grid-cols-2">
        {#each data.summaries as summary (summary.repository.id)}
          <RepositoryCard {summary} />
        {/each}
      </div>
      {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
    {/if}
  </section>

  <Dialog.Root bind:open={connectOpen}>
    <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
      <Dialog.Header>
        <Dialog.Title>Connect repository</Dialog.Title>
        <Dialog.Description>Choose a GitHub App and repository, then enable Ayni to configure its runtime.</Dialog.Description>
      </Dialog.Header>

      {#if options.error}<Button variant="outline" onclick={() => { void loadOptions(); }}>Try again</Button>{/if}
      {#if loadingOptions}<p class="text-sm text-muted-foreground" role="status">Loading GitHub repositories…</p>{/if}
      {#if options.installations.length}
        <div class="space-y-5">
          <div class="space-y-2">
            <Label for="github-app">GitHub App</Label>
            <ThemedSelect id="github-app" value={options.selectedAppName} options={[...new Set(options.installations.map(item => item.appName))].map(appName => ({ value: appName, label: appName }))} onValueChange={value => { void loadOptions(value); }} disabled={loadingOptions} required />
          </div>
          <div class="grid gap-5 sm:grid-cols-2">
            <div class="space-y-2">
              <Label for="installationId">Owner</Label>
              <ThemedSelect id="installationId" value={options.selectedInstallationId} options={options.installations.filter(item => item.appName === options.selectedAppName).map(item => ({ value: item.installationId, label: item.owner }))} onValueChange={value => { void loadOptions(options.selectedAppName, value); }} disabled={loadingOptions} required />
            </div>
            <div class="space-y-2">
              <Label for="repository">Repository</Label>
              <ThemedSelect id="repository" name="repository" form="connect-repository" bind:value={selectedRepository} options={options.repositories.map(item => ({ value: item.name, label: item.name }))} placeholder="Select a repository" required disabled={loadingOptions || !options.repositories.length} />
            </div>
          </div>
        </div>
        <form id="connect-repository" method="POST" action="?/connect" class="space-y-5" use:enhance={() => { connecting = true; return async ({ result, update }) => { connecting = false; await update(); if(result.type === "success") { connectOpen = false; selectedRepository = ""; } }; }}>
          <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
          <input type="hidden" name="installationId" value={options.selectedInstallationId} />
          <input type="hidden" name="owner" value={selectedOwner} />
          <input type="hidden" name="useAyni" value={useAyni ? "true" : "false"} />
          <div class="grid gap-4 sm:grid-cols-[auto_1fr]">
            <div class="flex h-9 items-center gap-2 sm:mt-6">
              <Checkbox.Root id="use-ayni" bind:checked={useAyni} disabled={connecting} onCheckedChange={checked => { if (!checked) selectedRegistryId = ""; }} />
              <Label for="use-ayni">Use Ayni</Label>
            </div>
            <div class="space-y-2">
              <Label for="connect-registry">GHCR connection</Label>
              <ThemedSelect id="connect-registry" name="registryConnectionId" bind:value={selectedRegistryId} options={[{ value: "", label: "Configure later" }, ...data.registryConnections.map(connection => ({ value: connection.id, label: connection.displayName }))]} disabled={connecting || !useAyni} />
              {#if imageDestination}<p class="break-all text-xs text-muted-foreground">Image destination: {imageDestination}</p>{:else if selectedRegistry}<p class="text-xs text-muted-foreground">Select a repository to preview its image destination.</p>{/if}
              {#if useAyni && data.registryError}

                <p class="text-sm text-muted-foreground">You can connect now and configure the runtime's image destination later.</p>
              {:else if useAyni && !data.registryConnections.length}
                <p class="text-sm text-muted-foreground">Add a ready <a href="/_old/container-registries/github" class="underline">GHCR connection</a> or configure the runtime's image destination later.</p>
              {/if}
            </div>
          </div>
          {#if !options.repositories.length}<p class="text-sm text-muted-foreground">No repositories are available to this installation.</p>{/if}
          <Dialog.Footer>
            <Button variant="outline" onclick={() => { connectOpen = false; }}>Cancel</Button>
            <SubmitButton pending={connecting} pendingLabel="Connecting…" disabled={loadingOptions || !selectedRepository}>Connect repository</SubmitButton>
          </Dialog.Footer>
        </form>
      {:else if !loadingOptions && !options.error}
        <p class="text-sm text-muted-foreground">No GitHub App installations are available. Set up GitHub in Source control first.</p>
        <Button href="/_old/source-control/github" variant="outline">Set up GitHub</Button>
      {/if}
    </Dialog.Content>
  </Dialog.Root>
</div>
