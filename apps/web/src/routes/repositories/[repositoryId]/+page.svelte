<script lang="ts">
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { enhance } from "$app/forms";
  import { ArrowLeft } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import EmptyState from "$lib/components/empty-state.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { statusTone, words } from "$lib/presentation.js";
  let { data, form }: PageProps = $props();
</script>

<svelte:head><title>{data.repositoryConnection?.displayName ?? "Repository unavailable"} · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <Button href="/repositories" variant="ghost"><ArrowLeft aria-hidden="true" /> Repository connections</Button>
  <PageHeader eyebrow="Repository connection" title={data.repositoryConnection?.displayName ?? "Repository unavailable"} description="Orchestrator inspects the default revision and reconciles its immutable runtime image without changing the repository." />
  {#if data.error}<Feedback kind="error" title="Repository data unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if form?.error}<Feedback kind="error" title="Runtime action failed" message={form.error.message} />{:else if form?.success}<Feedback kind="success" title="Runtime action requested" message={form.success} />{/if}
  {#if data.repositoryConnection}
    <Card.Root><Card.Header><div class="flex items-start justify-between"><div><Card.Title>{data.repositoryConnection.provider.owner}/{data.repositoryConnection.provider.name}</Card.Title><Card.Description>{data.repositoryConnection.provider.htmlUrl}</Card.Description></div><StatusBadge status={data.repositoryConnection.status} tone={statusTone(data.repositoryConnection.status)} /></div></Card.Header><Card.Content><p class="text-sm"><span class="text-muted-foreground">Default branch</span> {data.repositoryConnection.defaultBranch}</p></Card.Content></Card.Root>
    <form method="POST" action="?/registry" use:enhance class="space-y-3 rounded-xl border bg-card p-6">
      <h2 class="text-lg font-semibold">Container registry</h2>
      <input type="hidden" name="idempotencyKey" value={data.registryKey}/><input type="hidden" name="resourceVersion" value={data.repositoryConnection.resourceVersion}/>
      <label for="registry-connection">GHCR connection</label>
      <ThemedSelect id="registry-connection" name="registryConnectionId" value={data.repositoryConnection.registryConnectionId ?? ""} options={data.registryConnections.map(c=>({value:c.id,label:c.displayName}))} required/>
      <label for="image-namespace">Image namespace</label><input id="image-namespace" name="imageNamespace" value={data.repositoryConnection.imageNamespace ?? ""} required class="block w-full rounded border bg-background p-2"/>
      <Button type="submit">Save registry selection</Button>
    </form>
    <Card.Root><Card.Header><Card.Title>Automatic runtime inspection</Card.Title><Card.Description>Every connection is inspected at its default revision. A supported <code>.ayni.toml</code> and <code>.ayni.lock</code> establish the immutable lock fingerprint used to resolve the runtime image.</Card.Description></Card.Header><Card.Content><p class="text-sm">Orchestrator first verifies the configured registry image. If no verified image matches the checked lock, it creates a dedicated Kubernetes build Job, then verifies the published artifact before making it available.</p></Card.Content></Card.Root>
    <section class="space-y-4"><div><h2 class="text-lg font-semibold">Runtime profiles</h2><p class="text-sm text-muted-foreground">Profiles bind the checked source snapshot, lock fingerprint, registry reference and verification status.</p></div>
      {#if data.runtimeProfiles.length === 0}<EmptyState title="Inspection pending" description="The connected repository is being inspected. Missing or unsupported Ayni configuration will appear here as a failure." />{:else}<div class="grid gap-4 lg:grid-cols-2">{#each data.runtimeProfiles as profile (profile.id)}<Card.Root><Card.Header><div class="flex justify-between"><div><Card.Title><a class="hover:underline" href={`/profiles/${encodeURIComponent(profile.id)}`}>{words(profile.kind)}</a></Card.Title><Card.Description>{profile.id}</Card.Description></div><StatusBadge status={profile.state} tone={statusTone(profile.state)} /></div></Card.Header><Card.Content class="space-y-3 text-sm"><p>Support: {words(profile.support)}</p>{#if profile.failure}<p class="text-destructive">{profile.failure.message}</p>{/if}<div class="flex flex-wrap gap-2"><form method="POST" action="?/sync" use:enhance><input type="hidden" name="profileId" value={profile.id} /><input type="hidden" name="idempotencyKey" value={data.runtimeKeys[profile.id]} /><Button type="submit" variant="outline">Sync runtime</Button></form><form method="POST" action="?/rebuild" use:enhance><input type="hidden" name="profileId" value={profile.id} /><input type="hidden" name="idempotencyKey" value={data.runtimeKeys[profile.id]} /><Button type="submit">Rebuild runtime</Button></form></div></Card.Content></Card.Root>{/each}</div>{/if}
    </section>
  {/if}
</div>
