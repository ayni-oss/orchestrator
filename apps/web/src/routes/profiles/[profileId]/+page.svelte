<script lang="ts">
  import { enhance } from "$lib/forms.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import PollStatus from "$lib/components/poll-status.svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { words, formatDate } from "$lib/presentation.js";

  let { data }: PageProps = $props();
  let syncForm = $state<HTMLFormElement | undefined>();
  let rebuildForm = $state<HTMLFormElement | undefined>();
  let busy = $state(false);
  const submit: SubmitFunction = () => {
    busy = true;
    return async ({ update }) => {
      try { await update(); } finally { busy = false; }
    };
  };
  function reconciliationValue(key: string, value: string) {
    if (["last-success", "next-retry"].includes(key) && /^\d+$/u.test(value)) {
      const timestamp = Number(value) * 1000;
      if (Number.isFinite(timestamp) && Math.abs(timestamp) <= 8.64e15) return formatDate(new Date(timestamp).toISOString());
    }
    return value;
  }
</script>

<svelte:head><title>Runtime profile · Ayni Orchestrator</title></svelte:head>
<div class="max-w-4xl space-y-6">

  {#if data.profile && data.repository}
    {@const profile = data.profile}
    {@const trustUpdate = ["merged", "verified"].includes(profile.reconciliation.find(entry => entry.key === "certificate-trust-state")?.value ?? "") ? undefined : profile.reconciliation.find(entry => entry.key === "certificate-trust-pr")}
    <Button href={`/repositories/${encodeURIComponent(data.repository.id)}`} variant="ghost">Back to {data.repository.displayName}</Button>
    <PageHeader title={words(profile.kind)} />
    <section class="space-y-4" aria-labelledby="runtime-identity-title">
      <div class="flex flex-wrap items-center justify-between gap-2"><h2 id="runtime-identity-title" class="text-lg font-semibold">Runtime</h2><PollStatus /></div>
      {#snippet certificate()}
        {#if profile.effectiveCertificate}
          {profile.effectiveCertificate.displayName} · {profile.effectiveCertificate.keyId} · revision {profile.effectiveCertificate.revision}{profile.certificateId ? "" : " (global default)"}
        {:else}<a href="/certificates" class="underline">Add a signing certificate</a>{/if}
      {/snippet}
      {#snippet trustLink()}
        {#if trustUpdate}<a href={trustUpdate.value} target="_blank" rel="noreferrer" class="underline">Review certificate trust update</a>{/if}
      {/snippet}
      <InformationCard title="Current runtime identity" status={profile.state} actions={[
        { label: "Sync", onSelect: () => syncForm?.requestSubmit(), disabled: busy },
        { label: "Rebuild", onSelect: () => rebuildForm?.requestSubmit(), disabled: busy || !profile.rebuildAvailable },
      ]}>
        <InformationTable rows={[
          { label: "Preparation", value: words(profile.preparationPhase) },
          ...(profile.support !== "operational" ? [{ label: "Support", value: "Custom runtime execution is not available in this release." }] : []),
          ...(profile.kind === "ayni_managed" ? [{ label: "Certificate", value: certificate }] : []),
          ...(data.revision ? [{ label: "Source snapshot", value: data.revision.repositoryRevision, monospace: true }] : []),
          ...(data.revision?.managed ? [
            { label: "Lock fingerprint", value: data.revision.managed.fingerprint, monospace: true },
            { label: "Registry repository", value: profile.desiredImageRepository || data.revision.managed.repository, monospace: true },
            { label: "Ayni version", value: data.revision.managed.ayniVersion },
          ] : profile.desiredImageRepository ? [{ label: "Registry repository", value: profile.desiredImageRepository, monospace: true }] : []),
          ...profile.reconciliation.filter(entry => entry.key !== "certificate-trust-pr"
            && !(entry.key === "default-sha" && entry.value === data.revision?.repositoryRevision)
            && !(entry.key === "lock-fingerprint" && entry.value === data.revision?.managed?.fingerprint))
            .map(entry => ({ label: words(entry.key.replaceAll("-", " ")), value: reconciliationValue(entry.key, entry.value), monospace: !["last-success", "next-retry"].includes(entry.key) })),
          ...(trustUpdate ? [{ label: "Certificate trust update", value: trustLink }] : []),
        ]} />
        {#each profile.findings as finding (finding.code)}<Feedback kind="warning" title={finding.message} message={finding.remediation} />{/each}
        {#if profile.failure && !profile.findings.length}
          <Feedback kind="warning" title={words(profile.failure.reason)} message={profile.failure.message} />
          {#if profile.failure.remediation}<p class="text-sm text-muted-foreground">{profile.failure.remediation}</p>{/if}
        {/if}
        <form hidden bind:this={syncForm} method="POST" action="?/reconcile" use:enhance={submit}><input type="hidden" name="idempotencyKey" value={data.keys.reconcile} /></form>
        <form hidden bind:this={rebuildForm} method="POST" action="?/reconcile" use:enhance={submit}><input type="hidden" name="idempotencyKey" value={data.keys.reconcile} /><input type="hidden" name="forceRebuild" value="true" /></form>
      </InformationCard>
    </section>
    <section class="space-y-4" aria-labelledby="runtime-environments-title">
      <h2 id="runtime-environments-title" class="text-lg font-semibold">Environments</h2>
      {#each data.artifacts ?? [] as artifact (artifact.id)}
        <InformationCard title={`Environment · ${artifact.platform.operatingSystem}/${artifact.platform.architecture}`} status={artifact.status}>
          <InformationTable rows={[
            { label: "Image repository", value: artifact.ociRepository, monospace: true },
            { label: "Digest", value: artifact.resolvedDigest || "Not resolved yet", monospace: true },
            ...(artifact.discoveryTag ? [{ label: "Discovery tag", value: artifact.discoveryTag, monospace: true }] : []),
            { label: "Verified", value: artifact.verifiedAt ? formatDate(artifact.verifiedAt) : "Not verified yet" },
            ...artifact.verifiedMetadata.map(entry => ({ label: words(entry.key.replaceAll("-", " ")), value: entry.value })),
          ]} />
          {#if artifact.failure}<Feedback kind="warning" title="Environment needs attention" message={artifact.failure.message} />{/if}
        </InformationCard>
      {:else}
        <InformationCard title="Environment" status="pending"><InformationTable rows={[{ label: "Image", value: "No verified environment is available yet." }]} /></InformationCard>
      {/each}
    </section>
  {/if}
</div>
