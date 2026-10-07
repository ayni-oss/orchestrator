<script lang="ts">
  import type { RepositoryOptions } from "$lib/repository-options.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { enhance } from "$app/forms";
  import { ArrowRight, Plus } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { statusTone } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let connecting = $state(false);
  let connectOpen = $state(false);
  let options = $state<RepositoryOptions>({ installations: [], selectedAppName: "", selectedInstallationId: "", repositories: [] });
  let loadingOptions = $state(false);
  let optionsRequest = 0;
  async function loadOptions(appName = "", installationId = "") {
    const sequence = ++optionsRequest;
    loadingOptions = true;
    selectedRepository = "";
    options.error = undefined;
    try {
      const query = new URLSearchParams({ appName, installationId });
      const response = await fetch(`/repositories/options?${query}`);
      const result = await response.json();
      if (sequence !== optionsRequest) return;
      if (response.ok) options = result;
      else options.error = result.error;
    } catch {
      if (sequence === optionsRequest) options.error = { status: 503, reason: "provider_unavailable", message: "GitHub options could not be loaded. Try again.", requestId: data.requestId, retryable: true };
    } finally {
      if (sequence === optionsRequest) loadingOptions = false;
    }
  }
  function openConnect() {
    connectOpen = true;
    void loadOptions();
  }
  let selectedRepository = $state("");
  const selectedOwner = $derived(options.installations.find(item => item.installationId === options.selectedInstallationId)?.owner ?? "");
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/repositories?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Repository connections · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Connectivity" title="Repository connections" description="Connect a repository and Orchestrator immediately inspects its default revision and resolves a verified runtime image.">
    {#snippet actions()}<Button onclick={openConnect}><Plus aria-hidden="true" /> Connect repository</Button>{/snippet}
  </PageHeader>
  {#if data.error}<Feedback kind="error" title="Connections unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Connection failed" message={form.error.message} requestId={form.error.requestId} />{:else if form?.success}<Feedback kind="success" title="Repository connected" message={form.success} />{/if}

  <section class="space-y-4">
    {#if data.repositoryConnections.length === 0}
      <EmptyState title="No repository connections" description="Connect an approved provider repository to inspect its committed Ayni configuration and runtime image." />
    {:else}
      <div class="grid gap-4 lg:grid-cols-2">
        {#each data.repositoryConnections as item (item.id)}
          {@const profiles = data.runtimeProfiles[item.id] ?? []}
          {@const runtime = profiles.find((profile) => profile.state === "ready") ?? profiles[0]}
          <Card.Root><Card.Header><div class="flex items-start justify-between gap-3"><div><Card.Title>{item.displayName}</Card.Title><Card.Description>{item.provider.owner}/{item.provider.name} · {item.defaultBranch}</Card.Description></div>{#if runtime}<StatusBadge status={runtime.state} tone={statusTone(runtime.state)} />{:else}<span class="rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">Inspection pending</span>{/if}</div></Card.Header><Card.Content class="text-sm">{#if runtime}<p>Runtime: <span class="text-muted-foreground">{runtime.state === "ready" ? "Verified and ready" : runtime.state.replaceAll("_", " ")}</span></p>{:else}<p class="text-muted-foreground">No runtime profile yet. Orchestrator is inspecting the committed Ayni configuration.</p>{/if}</Card.Content><Card.Footer><Button href={`/repositories/${encodeURIComponent(item.id)}`} variant="outline">Runtime status <ArrowRight aria-hidden="true" /></Button></Card.Footer></Card.Root>
        {/each}
      </div>
      {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
    {/if}
  </section>

  <Dialog.Root bind:open={connectOpen}>
    <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
      <Dialog.Header>
        <Dialog.Title>Connect repository</Dialog.Title>
        <Dialog.Description>Choose a GitHub App, repository, and container registry destination.</Dialog.Description>
      </Dialog.Header>
      {#if form?.error}<Feedback kind="error" title="Connection failed" message={form.error.message} requestId={form.error.requestId} />{/if}
      {#if options.error}<Feedback kind="error" title="GitHub unavailable" message={options.error.message} requestId={options.error.requestId} /><Button variant="outline" onclick={() => { void loadOptions(); }}>Try again</Button>{/if}
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
          <div class="space-y-2">
            <Label for="registry-destination">Container registry destination</Label>
            <ThemedSelect id="registry-destination" name="registryDestination" value="ghcr.io" options={[{ value: "ghcr.io", label: "GitHub Container Registry" }, { value: "docker.io", label: "Docker Hub — Not available", disabled: true }, { value: "ecr", label: "Amazon ECR — Not available", disabled: true }]} required />
            <p class="text-xs text-muted-foreground">{#if selectedRepository}Image destination: ghcr.io/{selectedOwner.toLowerCase()}/{selectedRepository.toLowerCase()}-ayni{:else}Select a repository to see its image destination.{/if}</p>
          </div>
          {#if !options.repositories.length}<p class="text-sm text-muted-foreground">No repositories are available to this installation.</p>{/if}
          <Dialog.Footer>
            <Button variant="outline" onclick={() => { connectOpen = false; }}>Cancel</Button>
            <SubmitButton pending={connecting} pendingLabel="Connecting…" disabled={loadingOptions || !selectedRepository}>Connect repository</SubmitButton>
          </Dialog.Footer>
        </form>
      {:else if !loadingOptions && !options.error}
        <p class="text-sm text-muted-foreground">No GitHub App installations are available. Set up GitHub in Source control first.</p>
        <Button href="/source-control" variant="outline">Go to Source control</Button>
      {/if}
    </Dialog.Content>
  </Dialog.Root>
</div>
