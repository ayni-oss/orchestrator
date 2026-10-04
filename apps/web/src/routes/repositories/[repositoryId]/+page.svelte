<script lang="ts">
  import { enhance } from "$app/forms";
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
  let { data, form }: PageProps = $props();
</script>

<svelte:head><title>{data.repositoryConnection?.displayName ?? "Repository unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <Button href="/repositories" variant="ghost"><ArrowLeft aria-hidden="true" /> Repository connections</Button>
  <PageHeader eyebrow="Repository connection" title={data.repositoryConnection?.displayName ?? "Repository unavailable"} description="Provider access and runtime-profile readiness are separate, independently reconciled resources." />
  {#if data.error}<Feedback kind="error" title="Repository data unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if data.repositoryConnection}
    <Card.Root><Card.Header><div class="flex items-start justify-between"><div><Card.Title>{data.repositoryConnection.provider.owner}/{data.repositoryConnection.provider.name}</Card.Title><Card.Description>{data.repositoryConnection.provider.htmlUrl}</Card.Description></div><StatusBadge status={data.repositoryConnection.status} tone={statusTone(data.repositoryConnection.status)} /></div></Card.Header><Card.Content><dl class="grid gap-4 text-sm sm:grid-cols-2"><div><dt class="text-muted-foreground">Connection ID</dt><dd><CodeValue value={data.repositoryConnection.id} /></dd></div><div><dt class="text-muted-foreground">Default branch</dt><dd>{data.repositoryConnection.defaultBranch}</dd></div><div><dt class="text-muted-foreground">Provider repository ID</dt><dd>{data.repositoryConnection.provider.repositoryId}</dd></div><div><dt class="text-muted-foreground">Updated</dt><dd>{formatDate(data.repositoryConnection.updatedAt)}</dd></div></dl></Card.Content></Card.Root>
    {#if form?.error}<Feedback kind="error" title="Onboarding needs attention" message={form.error.message} />{/if}
    <Card.Root><Card.Header><Card.Title>Integrate with Ayni Managed</Card.Title><Card.Description>Review the environment build and assessment workflows before opening a setup PR.</Card.Description></Card.Header><Card.Content class="space-y-4">
      <p class="text-sm">The existing App needs contents, workflows and pull-request write access; Actions read access is used for assessments. Signing keys and registry access are configured by your operator outside this UI.</p>
      <form method="POST" action="?/inspect" use:enhance><Button type="submit" variant="outline">Inspect repository</Button></form>
      {#if form?.inspection}
        {@const inspection=form.inspection}
        <dl class="space-y-2 text-sm"><div><dt>Default-branch revision</dt><dd><CodeValue value={inspection.head} /></dd></div><div><dt>Policy</dt><dd>{inspection.policyPresent ? "Present" : "Missing — commit .ayni.toml"}</dd></div><div><dt>Managed lock</dt><dd>{inspection.lockCompatible ? "Compatible" : "Generate and commit a lock with the supported Ayni release"}</dd></div><div><dt>Certificate trust policy</dt><dd>{inspection.trustPinned ? "Pinned policy observed; certificate verification is still required" : "Configure a public trust key and regenerate the lock before building"}</dd></div><div><dt>Environment package</dt><dd><CodeValue value={inspection.repository} /></dd></div></dl>
        <p class="text-sm">After merge, GitHub Actions builds the signed environment. The operator must supply the matching signing configuration. A profile is ready only after Orchestrator verifies the produced image.</p>
        {#each inspection.workflows as workflow (workflow.path)}<details class="rounded-lg border p-3"><summary>{workflow.path}</summary><pre class="mt-3 overflow-x-auto text-xs">{workflow.contents}</pre></details>{/each}
        <form method="POST" action="?/onboard" use:enhance><input type="hidden" name="idempotencyKey" value={data.idempotencyKey} /><Button type="submit" disabled={!inspection.policyPresent || !inspection.lockCompatible}>Set up runtime</Button></form>
      {/if}
      <details><summary>Custom Runtime</summary><p class="mt-2 text-sm text-muted-foreground">Orchestrator supports a separate runtime model for repositories without Ayni. Custom Runtime execution is not available in this release.</p></details>
    </Card.Content></Card.Root>
    <section class="space-y-4"><div><h2 class="text-lg font-semibold">Runtime profiles</h2><p class="text-sm text-muted-foreground">A profile becomes ready only after its immutable runtime artifact is verified.</p></div>
      {#if data.runtimeProfiles.length === 0}<EmptyState title="No runtime profile" description="Inspect the repository to prepare its first setup PR." />{:else}<div class="grid gap-4 lg:grid-cols-2">{#each data.runtimeProfiles as profile (profile.id)}<Card.Root><Card.Header><div class="flex justify-between"><div><Card.Title><a class="hover:underline" href={`/profiles/${encodeURIComponent(profile.id)}`}>{words(profile.kind)}</a></Card.Title><Card.Description>{profile.id}</Card.Description></div><StatusBadge status={profile.state} tone={statusTone(profile.state)} /></div></Card.Header><Card.Content class="text-sm"><p>Support: {words(profile.support)}</p>{#if profile.activeRevisionId}<p>Revision <code>{profile.activeRevisionId}</code></p>{/if}{#if profile.failure}<p class="text-destructive">{profile.failure.message}</p>{/if}</Card.Content></Card.Root>{/each}</div>{/if}
    </section>
  {/if}
</div>
