<script lang="ts">
  import { enhance } from "$lib/forms.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { ExternalLink, Trash2 } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import RuntimeOverview from "$lib/components/runtime-overview.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { words } from "$lib/presentation.js";
  let { data }: PageProps = $props();
  let confirmation = $state("");
  let analysisForm = $state<HTMLFormElement | undefined>();
  let busy = $state(false);
  const remove: SubmitFunction = () => {
    busy = true;
    return async ({ update }) => {
      try { await update({ reset: false }); } finally { busy = false; }
    };
  };
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
  <Button href="/repositories" variant="ghost">Repositories</Button>

  {#if data.repositoryConnection}
    {@const repository = data.repositoryConnection}
    {@const showOnboarding = repository.useAyni !== false && (!data.runtimeProfiles.length || data.runtimeProfiles.some(profile => profile.kind === "ayni_managed"))}
    <header class="space-y-3">
      <div class="flex flex-wrap items-center gap-3">
        <h1 class="text-2xl font-semibold tracking-tight sm:text-3xl">{repository.displayName}</h1>
      </div>
    </header>
    {#if repository.failure}<Feedback kind="warning" title="Repository needs attention" message={repository.failure.message} />{/if}
    <section class="space-y-4" aria-labelledby="repository-information-title">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 id="repository-information-title" class="text-lg font-semibold">Information</h2>
      </div>
      <div class={["grid gap-4", showOnboarding && "lg:grid-cols-2"]}>
        {#snippet repositoryUrl()}
          <a href={repository.provider.htmlUrl} target="_blank" rel="noreferrer" class="inline-flex max-w-full items-center gap-2 hover:underline"><span class="min-w-0 [overflow-wrap:anywhere]">{repository.provider.htmlUrl}</span><ExternalLink class="size-4 shrink-0" aria-hidden="true" /></a>
        {/snippet}
        <InformationCard title="Repository" status={repository.status}>
          <InformationTable rows={[
            { label: "Full name", value: `${repository.provider.owner}/${repository.provider.name}` },
            { label: "Default branch", value: repository.defaultBranch },
            { label: "URL", value: repositoryUrl },
          ]} />
        </InformationCard>
        {#if showOnboarding}
          <InformationCard title="Ayni onboarding" status={repository.onboardingStatus === "enrolled" ? "analysis_complete" : repository.onboardingStatus || "pending"} tone={repository.onboardingStatus === "enrolled" ? "success" : "neutral"} actions={[{ label: repository.onboardingStatus === "failed" ? "Retry analysis" : "Analyze repository", onSelect: () => analysisForm?.requestSubmit() }]}>
            <InformationTable rows={[
              { label: "Source commit", value: repository.onboardingRevision || "Not available yet", monospace: true },
              ...(repository.onboardingReason ? [{ label: "Next step", value: remediation[repository.onboardingReason] ?? words(repository.onboardingReason) }] : []),
            ]} />
            <form bind:this={analysisForm} method="POST" action="?/analyze" use:enhance>
              <input type="hidden" name="idempotencyKey" value={data.analyzeKey} />
            </form>
          </InformationCard>
        {/if}
      </div>
      {#if repository.useAyni === false}
        <form method="POST" action="?/analyze" use:enhance>
          <input type="hidden" name="idempotencyKey" value={data.analyzeKey} />
          <Button type="submit" variant="outline">Set up Ayni</Button>
        </form>
      {/if}
    </section>
    <RuntimeOverview profiles={data.runtimeProfiles} revisions={data.runtimeRevisions} repository={repository} connections={data.registryConnections} certificates={data.certificates} keys={data.runtimeKeys} />
    <section class="space-y-4" aria-labelledby="repository-danger-zone-title">
      <h2 id="repository-danger-zone-title" class="text-lg font-semibold">Danger zone</h2>
      <InformationCard title="Remove repository" danger>
        <InformationTable rows={[
          { label: "Removes", value: "This connection, onboarding results, and all runtime configuration. Reconnect the repository to start onboarding again." },
          { label: "Keeps", value: "Your GitHub repository, registry images, integrations, and signing certificates." },
          { label: "Blocked by", value: "Projects, retained tasks, or active preparation referencing this repository." },
        ]} />

        <form method="POST" action="?/remove" use:enhance={remove} class="space-y-4">
          <input type="hidden" name="idempotencyKey" value={data.removeKey} />
          <input type="hidden" name="resourceVersion" value={repository.resourceVersion} />
          <div class="space-y-2">
            <Label for="remove-repository-confirmation">Enter {repository.displayName} to confirm</Label>
            <Input id="remove-repository-confirmation" name="confirmation" bind:value={confirmation} autocomplete="off" disabled={busy} required />
          </div>
          <div class="flex justify-end">
            <SubmitButton variant="destructive" pending={busy} pendingLabel="Removing…" disabled={confirmation !== repository.displayName}>
              <Trash2 aria-hidden="true" /> Remove repository
            </SubmitButton>
          </div>
        </form>
      </InformationCard>
    </section>
  {/if}
</div>
