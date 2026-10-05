<script lang="ts">
  import { enhance } from "$app/forms";
  import { ArrowRight, FolderGit2, Plus, PlugZap } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { statusTone } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let connecting = $state(false);
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/repositories?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Repository connections · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Connectivity" title="Repository connections" description="Connect a repository and Orchestrator immediately inspects its default revision and resolves a verified runtime image.">
    {#snippet actions()}<Button href="/setup/github" variant="outline"><PlugZap aria-hidden="true" /> Configure GitHub App</Button><Button href="#connect"><Plus aria-hidden="true" /> Connect repository</Button>{/snippet}
  </PageHeader>
  {#if data.error}<Feedback kind="error" title="Connections unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Connection failed" message={form.error.message} requestId={form.error.requestId} />{:else if form?.success}<Feedback kind="success" title="Repository connected" message={form.success} />{/if}

  <section class="space-y-4">
    {#if data.repositoryConnections.length === 0}
      <EmptyState title="No repository connections" description="Connect an approved provider repository to inspect its committed Ayni configuration and runtime image." />
    {:else}
      <div class="grid gap-4 lg:grid-cols-2">
        {#each data.repositoryConnections as item (item.id)}
          <Card.Root><Card.Header><div class="flex items-start justify-between gap-3"><div><Card.Title>{item.displayName}</Card.Title><Card.Description>{item.provider.owner}/{item.provider.name} · {item.defaultBranch}</Card.Description></div><StatusBadge status={item.status} tone={statusTone(item.status)} /></div></Card.Header><Card.Content class="text-sm"><p>GitHub repository ID <code>{item.provider.repositoryId}</code></p><p class="text-muted-foreground">Orchestrator keeps source access separate from registry credentials and signing material.</p></Card.Content><Card.Footer><Button href={`/repositories/${encodeURIComponent(item.id)}`} variant="outline">Runtime status <ArrowRight aria-hidden="true" /></Button></Card.Footer></Card.Root>
        {/each}
      </div>
      {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
    {/if}
  </section>

  <Card.Root id="connect"><Card.Header><div class="flex items-center gap-3"><FolderGit2 class="size-5" /><div><Card.Title>Connect repository</Card.Title><Card.Description>Select the GitHub App installation and repository. Repository identity and branch settings are filled in from GitHub.</Card.Description></div></div></Card.Header><Card.Content class="space-y-5">
    {#if data.installations?.length}
    {@const appNames = [...new Set(data.installations.map((item) => item.appName))]}
    <form method="GET" action="/repositories" class="grid gap-5 sm:grid-cols-2">
      <div class="space-y-2"><Label for="github-app">GitHub App</Label><select id="github-app" class="h-9 w-full rounded-lg border bg-background px-3 text-sm" value={appNames[0]} disabled={appNames.length === 1} aria-label="GitHub App">{#each appNames as appName}<option value={appName}>{appName}</option>{/each}</select></div>
      <div class="space-y-2"><Label for="installationId">GitHub owner</Label><select id="installationId" name="installationId" class="h-9 w-full rounded-lg border bg-background px-3 text-sm" value={data.selectedInstallationId} onchange={(event) => event.currentTarget.form?.requestSubmit()} required><option value="" disabled>Select an owner</option>{#each data.installations as installation (installation.installationId)}<option value={installation.installationId}>{installation.owner}</option>{/each}</select></div>
    </form>
    {#if data.repositories?.length}
    <form method="POST" action="?/connect" class="grid gap-5 sm:grid-cols-2" use:enhance={() => { connecting = true; return async ({ update }) => { connecting = false; await update(); }; }}>
      <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
      <input type="hidden" name="installationId" value={data.selectedInstallationId} />
      <input type="hidden" name="owner" value={data.installations.find((item) => item.installationId === data.selectedInstallationId)?.owner ?? ""} />
      <div class="space-y-2"><Label for="repository">Repository</Label><select id="repository" name="repository" class="h-9 w-full rounded-lg border bg-background px-3 text-sm" required><option value="" disabled selected>Select a repository</option>{#each data.repositories as repository (`${repository.owner}/${repository.name}`)}<option value={repository.name}>{repository.name}</option>{/each}</select></div>
      <div class="flex items-end"><SubmitButton pending={connecting} pendingLabel="Connecting…">Connect repository</SubmitButton></div>
    </form>
    {:else}<p class="text-sm text-muted-foreground">No repositories are available to this installation.</p>{/if}
    {:else if data.backend === "connected"}<p class="text-sm text-muted-foreground">No GitHub App installations are available. Configure the App and grant it access to an owner.</p>{/if}
  </Card.Content></Card.Root>
</div>
