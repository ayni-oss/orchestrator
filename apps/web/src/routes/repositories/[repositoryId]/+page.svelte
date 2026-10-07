<script lang="ts">
  import { ArrowLeft, ExternalLink } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import RuntimeOverview from "$lib/components/runtime-overview.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { statusTone } from "$lib/presentation.js";
  let { data, form }: PageProps = $props();
</script>

<svelte:head><title>{data.repositoryConnection?.displayName ?? "Repository unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="max-w-4xl space-y-6">
  <Button href="/repositories" variant="ghost"><ArrowLeft aria-hidden="true" /> Repositories</Button>
  {#if data.error}<Feedback kind="error" title="Repository data unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if data.repositoryConnection}
    {@const repository = data.repositoryConnection}
    <header class="space-y-3">
      <div class="flex flex-wrap items-center gap-3">
        <h1 class="text-2xl font-semibold tracking-tight sm:text-3xl"><a href={repository.provider.htmlUrl} target="_blank" rel="noreferrer" class="inline-flex items-center gap-2 hover:underline">{repository.displayName}<ExternalLink class="size-4" aria-hidden="true" /></a></h1>
        <StatusBadge status={repository.status} tone={statusTone(repository.status)} />
      </div>
      <p class="text-sm text-muted-foreground">Default branch <span class="font-medium text-foreground">{repository.defaultBranch}</span></p>
    </header>
    {#if repository.failure}<Feedback kind="warning" title="Repository needs attention" message={repository.failure.message} />{/if}
    <RuntimeOverview profiles={data.runtimeProfiles} revisions={data.runtimeRevisions} repository={repository} connections={data.registryConnections} keys={data.runtimeKeys} error={form?.error?.message} />
  {/if}
</div>
