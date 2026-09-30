<script lang="ts">
  import { ArrowLeft } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import CodeValue from "$lib/components/code-value.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { formatDate, statusTone, words } from "$lib/presentation.js";
  let { data }: PageProps = $props();
</script>

<svelte:head><title>{data.repositoryConnection?.displayName ?? "Repository unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <Button href="/repositories" variant="ghost"><ArrowLeft aria-hidden="true" /> Repository connections</Button>
  <PageHeader eyebrow="Repository connection" title={data.repositoryConnection?.displayName ?? "Repository unavailable"} description="Provider access and runtime-profile readiness are separate, independently reconciled resources." />
  {#if data.error}<Feedback kind="error" title="Repository data unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if data.repositoryConnection}
    <Card.Root><Card.Header><div class="flex items-start justify-between"><div><Card.Title>{data.repositoryConnection.provider.owner}/{data.repositoryConnection.provider.name}</Card.Title><Card.Description>{data.repositoryConnection.provider.htmlUrl}</Card.Description></div><StatusBadge status={data.repositoryConnection.status} tone={statusTone(data.repositoryConnection.status)} /></div></Card.Header><Card.Content><dl class="grid gap-4 text-sm sm:grid-cols-2"><div><dt class="text-muted-foreground">Connection ID</dt><dd><CodeValue value={data.repositoryConnection.id} /></dd></div><div><dt class="text-muted-foreground">Default branch</dt><dd>{data.repositoryConnection.defaultBranch}</dd></div><div><dt class="text-muted-foreground">Provider repository ID</dt><dd>{data.repositoryConnection.provider.repositoryId}</dd></div><div><dt class="text-muted-foreground">Updated</dt><dd>{formatDate(data.repositoryConnection.updatedAt)}</dd></div></dl></Card.Content></Card.Root>
    <section class="space-y-4"><div><h2 class="text-lg font-semibold">Runtime profiles</h2><p class="text-sm text-muted-foreground">A profile becomes ready only after its immutable runtime artifact is verified.</p></div>
      {#if data.runtimeProfiles.length === 0}<EmptyState title="No runtime profile" description="Ayni Managed onboarding is delivered by the profile lifecycle implementation." />{:else}<div class="grid gap-4 lg:grid-cols-2">{#each data.runtimeProfiles as profile}<Card.Root><Card.Header><div class="flex justify-between"><div><Card.Title>{words(profile.kind)}</Card.Title><Card.Description>{profile.id}</Card.Description></div><StatusBadge status={profile.state} tone={statusTone(profile.state)} /></div></Card.Header><Card.Content class="text-sm"><p>Support: {words(profile.support)}</p>{#if profile.activeRevisionId}<p>Revision <code>{profile.activeRevisionId}</code></p>{/if}{#if profile.failure}<p class="text-destructive">{profile.failure.message}</p>{/if}</Card.Content></Card.Root>{/each}</div>{/if}
    </section>
  {/if}
</div>
