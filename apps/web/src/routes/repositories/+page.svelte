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
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { statusTone } from "$lib/presentation.js";

  let { data, form }: PageProps = $props();
  let connecting = $state(false);
  const nextPageHref = $derived(data.nextPageToken === undefined ? undefined : `/repositories?pageToken=${encodeURIComponent(data.nextPageToken)}`);
</script>

<svelte:head><title>Repository connections · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8" data-bff-status={data.backend}>
  <PageHeader eyebrow="Connectivity" title="Repository connections" description="Connect provider repositories independently from runtime-profile onboarding and readiness.">
    {#snippet actions()}<Button href="/setup/github" variant="outline"><PlugZap aria-hidden="true" /> Configure GitHub App</Button><Button href="#connect"><Plus aria-hidden="true" /> Connect repository</Button>{/snippet}
  </PageHeader>
  {#if data.error}<Feedback kind="error" title="Connections unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Connection failed" message={form.error.message} requestId={form.error.requestId} />{:else if form?.success}<Feedback kind="success" title="Repository connected" message={form.success} />{/if}

  <section class="space-y-4">
    {#if data.repositoryConnections.length === 0}
      <EmptyState title="No repository connections" description="Connect an approved provider repository before creating a runtime profile." />
    {:else}
      <div class="grid gap-4 lg:grid-cols-2">
        {#each data.repositoryConnections as item}
          <Card.Root><Card.Header><div class="flex items-start justify-between gap-3"><div><Card.Title>{item.displayName}</Card.Title><Card.Description>{item.provider.owner}/{item.provider.name} · {item.defaultBranch}</Card.Description></div><StatusBadge status={item.status} tone={statusTone(item.status)} /></div></Card.Header><Card.Content class="text-sm"><p>GitHub repository ID <code>{item.provider.repositoryId}</code></p><p class="text-muted-foreground">Installation {item.provider.installationId}</p></Card.Content><Card.Footer><Button href={`/repositories/${encodeURIComponent(item.id)}`} variant="outline">Profiles and identity <ArrowRight aria-hidden="true" /></Button></Card.Footer></Card.Root>
        {/each}
      </div>
      {#if nextPageHref}<div class="flex justify-end"><Button href={nextPageHref} variant="outline">Next page</Button></div>{/if}
    {/if}
  </section>

  <Card.Root id="connect"><Card.Header><div class="flex items-center gap-3"><FolderGit2 class="size-5" /><div><Card.Title>Connect repository</Card.Title><Card.Description>GitHub-specific identity remains inside the provider boundary.</Card.Description></div></div></Card.Header><Card.Content>
    <form method="POST" action="?/connect" class="grid gap-5 sm:grid-cols-2" use:enhance={() => { connecting = true; return async ({ update }) => { connecting = false; await update(); }; }}>
      <input type="hidden" name="idempotencyKey" value={data.idempotencyKey} />
      <div class="space-y-2"><Label for="provider-repository-id">GitHub repository ID</Label><Input id="provider-repository-id" name="repositoryId" required inputmode="numeric" pattern="[1-9][0-9]*" /></div>
      <div class="space-y-2"><Label for="installation-id">GitHub App installation ID</Label><Input id="installation-id" name="installationId" required inputmode="numeric" pattern="[1-9][0-9]*" /></div>
      <div class="space-y-2"><Label for="owner">Owner</Label><Input id="owner" name="owner" required maxlength={100} /></div>
      <div class="space-y-2"><Label for="repository">Repository</Label><Input id="repository" name="repository" required maxlength={100} /></div>
      <div class="space-y-2"><Label for="display-name">Display name</Label><Input id="display-name" name="displayName" required maxlength={256} placeholder="owner/repository" /></div>
      <div class="space-y-2"><Label for="default-branch">Default branch</Label><Input id="default-branch" name="defaultBranch" required maxlength={255} value="main" /></div>
      <div class="sm:col-span-2"><SubmitButton pending={connecting} pendingLabel="Connecting…">Connect repository</SubmitButton></div>
    </form>
  </Card.Content></Card.Root>
</div>
