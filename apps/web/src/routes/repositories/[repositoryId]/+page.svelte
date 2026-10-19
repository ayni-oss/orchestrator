<script lang="ts">
  import WorkspaceShell from "$lib/components/workspace-shell.svelte";
  import { enhance } from "$lib/forms.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { ExternalLink, Trash2 } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import RuntimeOverview from "$lib/components/runtime-overview.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { statusTone, words } from "$lib/presentation.js";
  let { data }: PageProps = $props();
  let confirmation = $state("");
  let analysisForm = $state<HTMLFormElement | undefined>();
  let busy = $state(false);
  const trustUpdates = $derived(data.runtimeProfiles.filter(profile => profile.kind === "ayni_managed")
    .map(profile => ({
      id: profile.id,
      certificate: profile.effectiveCertificate?.displayName ?? "Signing certificate",
      state: profile.reconciliation.find(entry => entry.key === "certificate-trust-state")?.value,
      href: ["merged", "verified"].includes(profile.reconciliation.find(entry => entry.key === "certificate-trust-state")?.value ?? "")
        ? undefined : profile.reconciliation.find(entry => entry.key === "certificate-trust-pr")?.value,
    }))
    .filter((update, index, updates) => (update.state || update.href)
      && (!update.href || updates.findIndex(other => other.href === update.href) === index)));
  const trustDescriptions: Record<string, string> = {
    pending: "Checking the repository's signing certificate trust.",
    propose: "Creating a pull request with the certificate trust policy and regenerated lock.",
    open: "Review and merge the certificate trust update before the runtime can be prepared.",
    merged: "The trust update was merged. Checking the repository before preparing the runtime.",
    verified: "The repository trusts the selected signing certificate.",
    closed: "The trust update was closed without merging. Retry runtime preparation to reopen the proposal.",
    failed: "The certificate trust update failed. Review the runtime findings and retry preparation.",
  };
  const onboardingStatus = $derived(
    trustUpdates.some(update => ["closed", "failed"].includes(update.state ?? "")) ? "setup_required"
      : trustUpdates.some(update => update.state === "open" || (!update.state && update.href)) ? "setup_pr_open"
      : trustUpdates.some(update => ["pending", "propose", "merged"].includes(update.state ?? "")) ? "awaiting_trust"
      : data.repositoryConnection?.onboardingStatus === "enrolled" ? "analysis_complete"
      : data.repositoryConnection?.onboardingStatus || "pending");
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
<WorkspaceShell {data} title="Repository">
<div class="mx-auto max-w-5xl space-y-6">
  <Button href="/repositories" variant="ghost">Repositories</Button>

  {#if data.repositoryConnection}
    {@const repository = data.repositoryConnection}
    {@const showOnboarding = repository.useAyni !== false && (!data.runtimeProfiles.length || data.runtimeProfiles.some(profile => profile.kind === "ayni_managed"))}
    <PageHeader flush eyebrow="Repository" title={repository.displayName} description="Connection, onboarding, and runtimes for this repository." />
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
          <InformationCard title="Ayni onboarding" status={onboardingStatus === "setup_pr_open" ? "Setup PR open" : onboardingStatus} tone={onboardingStatus === "analysis_complete" ? "success" : statusTone(onboardingStatus)} actions={[{ label: repository.onboardingStatus === "failed" ? "Retry analysis" : "Analyze repository", onSelect: () => analysisForm?.requestSubmit() }]}>
            <InformationTable rows={[
              { label: "Source commit", value: repository.onboardingRevision || "Not available yet", monospace: true },
              ...(repository.onboardingReason ? [{ label: "Next step", value: remediation[repository.onboardingReason] ?? words(repository.onboardingReason) }] : []),
            ]} />
            {#each trustUpdates as update (update.id)}
              <div class="space-y-2 text-sm">
                <InformationTable rows={[
                  { label: "Certificate", value: update.certificate },
                  { label: "Certificate trust", value: words(update.state ?? "open") },
                ]} />
                <p class="text-muted-foreground">{trustDescriptions[update.state ?? "open"] ?? words(update.state ?? "open")}</p>
                {#if update.href}<a href={update.href} target="_blank" rel="noreferrer" class="inline-flex items-center gap-2 underline">Review certificate trust update<ExternalLink class="size-4 shrink-0" aria-hidden="true" /></a>{/if}
              </div>
            {/each}
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
  {:else}<Feedback kind="error" title="Repository unavailable" message={data.error?.message ?? "This repository could not be loaded."} />{/if}
</div>

</WorkspaceShell>
