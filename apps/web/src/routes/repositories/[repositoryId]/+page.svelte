<script lang="ts">
  import { enhance } from "$app/forms";
  import { mergeProps } from "bits-ui";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { ArrowLeft, ExternalLink, Trash2 } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import Feedback from "$lib/components/feedback.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import RuntimeOverview from "$lib/components/runtime-overview.svelte";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import SubmitButton from "$lib/components/submit-button.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { statusTone, words } from "$lib/presentation.js";
  let { data, form }: PageProps = $props();
  let editOpen = $state(false);
  let confirmation = $state("");
  let editVersion = $state("");
  let busy = $state(false);
  let removalError = $state("");
  function editRepository() {
    confirmation = "";
    removalError = "";
    editVersion = data.repositoryConnection?.resourceVersion ?? "";
  }
  const remove: SubmitFunction = () => {
    busy = true;
    removalError = "";
    return async ({ result, update }) => {
      busy = false;
      if (result.type === "failure") {
        const error = result.data?.error as { message?: string } | undefined;
        removalError = error?.message ?? "Repository removal failed. Try again.";
      } else if (result.type === "error") {
        removalError = "Repository removal failed. Try again.";
      }
      await update({ reset: false });
    };
  };
  const remediation: Record<string, string> = {
    policy_missing: "Commit .ayni.toml, then retry analysis.",
    lock_missing: "Generate and commit .ayni.lock with Ayni 0.15.0, then retry analysis.",
    runtime_version_mismatch: "Regenerate and commit .ayni.lock with the selected Ayni version, then retry analysis.",
    runtime_version_unsupported: "Update the repository policy and lock using supported Ayni 0.15.0 tooling, then retry analysis.",
    runtime_lock_contract_unsupported: "Validate and update the committed policy and lock using Ayni 0.15.0, then retry analysis.",
    repository_lock_invalid: "Repair the committed lock with matching Ayni tooling, then retry analysis.",
    provider_access_required: "Restore GitHub App access to this repository, then retry analysis.",
    provider_temporarily_unavailable: "GitHub is temporarily unavailable. Retry analysis after access recovers.",
    repository_provider_unconfigured: "Configure the GitHub App under Source control, then retry analysis.",
  };
</script>

<svelte:head><title>{data.repositoryConnection?.displayName ?? "Repository unavailable"} · Ayni Orchestrator</title></svelte:head>
<Dialog.Root bind:open={editOpen}>
<div class="max-w-4xl space-y-6">
  <Button href="/repositories" variant="ghost"><ArrowLeft aria-hidden="true" /> Repositories</Button>
  {#if data.error}<Feedback kind="error" title="Repository data unavailable" message={data.error.message} requestId={data.error.requestId} />{/if}
  {#if data.repositoryConnection}
    {@const repository = data.repositoryConnection}
    <header class="space-y-3">
      <div class="flex flex-wrap items-center gap-3">
        <h1 class="text-2xl font-semibold tracking-tight sm:text-3xl"><a href={repository.provider.htmlUrl} target="_blank" rel="noreferrer" class="inline-flex items-center gap-2 hover:underline">{repository.displayName}<ExternalLink class="size-4" aria-hidden="true" /></a></h1>
        <StatusBadge status={repository.status} tone={statusTone(repository.status)} />
        <Dialog.Trigger>
          {#snippet child({ props })}
            <Button {...mergeProps(props, { onclick: editRepository })} variant="outline" size="sm" class="ml-auto">Edit</Button>
          {/snippet}
        </Dialog.Trigger>
      </div>
      <p class="text-sm text-muted-foreground">Default branch <span class="font-medium text-foreground">{repository.defaultBranch}</span></p>
    </header>
    {#if repository.failure}<Feedback kind="warning" title="Repository needs attention" message={repository.failure.message} />{/if}
    <section class="space-y-3 rounded-lg border p-4" aria-label="Repository onboarding">
      <h2 class="font-semibold">Onboarding</h2>
      <p>{repository.useAyni === false ? "Ayni is not enabled for this repository." : repository.onboardingStatus === "enrolled" ? "Analysis complete — runtime preparation below" : words(repository.onboardingStatus || "pending")}</p>
      {#if repository.ayniVersion}<p class="text-sm text-muted-foreground">Selected Ayni version {repository.ayniVersion}</p>{/if}
      {#if repository.onboardingReason}<p class="text-sm text-muted-foreground">{remediation[repository.onboardingReason] ?? words(repository.onboardingReason)}</p>{/if}
      {#if repository.onboardingRevision}<p class="break-all text-xs text-muted-foreground">Source commit {repository.onboardingRevision}</p>{/if}
      <form method="POST" action="?/analyze" use:enhance>
        <input type="hidden" name="idempotencyKey" value={data.analyzeKey} />
        <Button type="submit" variant="outline">{repository.useAyni === false ? "Set up Ayni" : repository.onboardingStatus === "failed" ? "Retry analysis" : "Analyze repository"}</Button>
      </form>
    </section>
    {#if repository.useAyni !== false}
    <RuntimeOverview profiles={data.runtimeProfiles} revisions={data.runtimeRevisions} repository={repository} connections={data.registryConnections} certificates={data.certificates} keys={data.runtimeKeys} error={form?.error?.message} />
    {/if}
  {/if}
</div>

  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-lg" showCloseButton={!busy} onInteractOutside={(event) => { if (busy) event.preventDefault(); }} onEscapeKeydown={(event) => { if (busy) event.preventDefault(); }}>
    <Dialog.Header>
      <Dialog.Title>Edit repository</Dialog.Title>
      <Dialog.Description>{data.repositoryConnection?.displayName ?? "Repository unavailable"}</Dialog.Description>
    </Dialog.Header>
    <section class="space-y-4 border-t pt-4" aria-labelledby="remove-repository-title">
      <h2 id="remove-repository-title" class="font-semibold">Remove repository</h2>
      <p class="text-sm text-muted-foreground">Remove this connection, its onboarding results, and all runtime configuration from Ayni Orchestrator. You can then reconnect the repository to start onboarding again.</p>
      <p class="text-sm text-muted-foreground">Your GitHub repository, registry images, integrations, and signing certificates are kept. Removal is blocked while projects, retained tasks, or active preparation reference this repository.</p>
      {#if removalError}<Feedback kind="error" title="Repository removal failed" message={removalError} />{/if}
      <form method="POST" action="?/remove" use:enhance={remove} class="space-y-4">
        <input type="hidden" name="idempotencyKey" value={data.removeKey} />
        <input type="hidden" name="resourceVersion" value={editVersion} />
        <div class="space-y-2">
          <Label for="remove-repository-confirmation">Enter {data.repositoryConnection?.displayName} to confirm</Label>
          <Input id="remove-repository-confirmation" name="confirmation" bind:value={confirmation} autocomplete="off" disabled={busy} required />
        </div>
        <Dialog.Footer>
          <Button variant="outline" disabled={busy} onclick={() => editOpen = false}>Cancel</Button>
          <SubmitButton variant="destructive" pending={busy} pendingLabel="Removing…" disabled={!data.repositoryConnection || confirmation !== data.repositoryConnection.displayName}>
            <Trash2 aria-hidden="true" /> Remove repository
          </SubmitButton>
        </Dialog.Footer>
      </form>
    </section>
  </Dialog.Content>
</Dialog.Root>
