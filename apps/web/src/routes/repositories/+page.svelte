<script lang="ts">
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
  let connectOpen = $state(data.connectDialogOpen);
  let selectedRepository = $state("");
  const selectedOwner = $derived(data.installations.find(item => item.installationId === data.selectedInstallationId)?.owner ?? "");
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/repositories?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Repository connections · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Connectivity" title="Repository connections" description="Connect a repository and Orchestrator immediately inspects its default revision and resolves a verified runtime image.">
    {#snippet actions()}<Button onclick={() => { connectOpen = true; }}><Plus aria-hidden="true" /> Connect repository</Button>{/snippet}
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
      {#if data.installations.length}
        <form method="GET" action="/repositories" class="space-y-5">
          <input type="hidden" name="connect" value="1" />
          <div class="space-y-2">
            <Label for="github-app">GitHub App</Label>
            <select id="github-app" name="appName" class="h-9 w-full rounded-lg border bg-background px-3 text-sm" value={data.selectedAppName} onchange={event => event.currentTarget.form?.requestSubmit()} required>
              {#each [...new Set(data.installations.map(item => item.appName))] as appName (appName)}<option value={appName}>{appName}</option>{/each}
            </select>
          </div>
          <div class="grid gap-5 sm:grid-cols-2">
            <div class="space-y-2">
              <Label for="installationId">Owner</Label>
              <select id="installationId" name="installationId" class="h-9 w-full rounded-lg border bg-background px-3 text-sm" value={data.selectedInstallationId} onchange={event => event.currentTarget.form?.requestSubmit()} required>
                {#each data.installations.filter(item => item.appName === data.selectedAppName) as installation (installation.installationId)}<option value={installation.installationId}>{installation.owner}</option>{/each}
              </select>
            </div>
            <div class="space-y-2">
              <Label for="repository">Repository</Label>
              <select id="repository" name="repository" form="connect-repository" bind:value={selectedRepository} class="h-9 w-full rounded-lg border bg-background px-3 text-sm" required disabled={!data.repositories.length}>
                <option value="" disabled>Select a repository</option>
                {#each data.repositories as repository (`${repository.owner}/${repository.name}`)}<option value={repository.name}>{repository.name}</option>{/each}
              </select>
            </div>
          </div>
        </form>
        <form id="connect-repository" method="POST" action="?/connect" class="space-y-5" use:enhance={() => { connecting = true; return async ({ result, update }) => { connecting = false; await update(); if(result.type === "success") { connectOpen = false; selectedRepository = ""; } }; }}>
          <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
          <input type="hidden" name="installationId" value={data.selectedInstallationId} />
          <input type="hidden" name="owner" value={selectedOwner} />
          <div class="space-y-2">
            <Label for="registry-destination">Container registry destination</Label>
            <select id="registry-destination" name="registryDestination" class="h-9 w-full rounded-lg border bg-background px-3 text-sm" required>
              <option value="ghcr.io">GitHub Container Registry</option>
              <option disabled>Docker Hub — Not available</option>
              <option disabled>Amazon ECR — Not available</option>
            </select>
            <p class="text-xs text-muted-foreground">{#if selectedRepository}Image destination: ghcr.io/{selectedOwner.toLowerCase()}/{selectedRepository.toLowerCase()}-ayni{:else}Select a repository to see its image destination.{/if}</p>
          </div>
          {#if !data.repositories.length}<p class="text-sm text-muted-foreground">No repositories are available to this installation.</p>{/if}
          <Dialog.Footer>
            <Button variant="outline" onclick={() => { connectOpen = false; }}>Cancel</Button>
            <SubmitButton pending={connecting} pendingLabel="Connecting…" disabled={!selectedRepository}>Connect repository</SubmitButton>
          </Dialog.Footer>
        </form>
      {:else}
        <p class="text-sm text-muted-foreground">No GitHub App installations are available. Set up GitHub in Source control first.</p>
        <Button href="/source-control" variant="outline">Go to Source control</Button>
      {/if}
    </Dialog.Content>
  </Dialog.Root>
</div>
