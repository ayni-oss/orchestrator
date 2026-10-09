<script lang="ts">
  import { enhance } from "$app/forms";
  import { ArrowLeft, ExternalLink } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import RuntimeOverview from "$lib/components/runtime-overview.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { statusTone, words } from "$lib/presentation.js";
  let { data, form }: PageProps = $props();
  const remediation: Record<string, string> = {
    policy_missing: "Commit .ayni.toml, then retry analysis.",
    lock_missing: "Generate and commit .ayni.lock with Ayni 0.15.0, then retry analysis.",
    runtime_version_unsupported: "Update the repository policy and lock using supported Ayni 0.15.0 tooling, then retry analysis.",
    runtime_lock_contract_unsupported: "Validate and update the committed policy and lock using Ayni 0.15.0, then retry analysis.",
    repository_lock_invalid: "Repair the committed lock with matching Ayni tooling, then retry analysis.",
    provider_access_required: "Restore GitHub App access to this repository, then retry analysis.",
    provider_temporarily_unavailable: "GitHub is temporarily unavailable. Retry analysis after access recovers.",
    repository_provider_unconfigured: "Configure the GitHub App under Source control, then retry analysis.",
  };
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
    <section class="space-y-3 rounded-lg border p-4" aria-label="Repository onboarding">
      <h2 class="font-semibold">Onboarding</h2>
      <p>{repository.onboardingStatus === "enrolled" ? "Analysis complete — runtime preparation below" : words(repository.onboardingStatus || "pending")}</p>
      {#if repository.onboardingReason}<p class="text-sm text-muted-foreground">{remediation[repository.onboardingReason] ?? words(repository.onboardingReason)}</p>{/if}
      {#if repository.onboardingRevision}<p class="break-all text-xs text-muted-foreground">Source commit {repository.onboardingRevision}</p>{/if}
      <form method="POST" action="?/analyze" use:enhance>
        <input type="hidden" name="idempotencyKey" value={data.analyzeKey} />
        <Button type="submit" variant="outline">{repository.onboardingStatus === "failed" ? "Retry analysis" : "Analyze repository"}</Button>
      </form>
    </section>
    <RuntimeOverview profiles={data.runtimeProfiles} revisions={data.runtimeRevisions} repository={repository} connections={data.registryConnections} certificates={data.certificates} keys={data.runtimeKeys} error={form?.error?.message} />
  {/if}
</div>
