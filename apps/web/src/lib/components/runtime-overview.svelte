<script lang="ts">
  import { notify } from "$lib/notifications.js";
  import { tick } from "svelte";
  import { enhance } from "$lib/forms.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type {RuntimeProfileDto,RepositoryConnectionDto} from "$lib/project-view.js";
  import { registryImageRepository } from "$lib/connection.js";
  import type { ConnectionView } from "$lib/connection.js";
  import type { CertificateView } from "$lib/certificate.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { Label } from "$lib/components/ui/label/index.js";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import PollStatus from "$lib/components/poll-status.svelte";
  import { statusTone, words } from "$lib/presentation.js";

  type Revision = { managed?: {fingerprint:string;repository:string} | undefined };
  let { profiles, revisions, repository, connections, certificates, keys }: {
    profiles:RuntimeProfileDto[]; revisions:Record<string,Revision | undefined>;
    repository:RepositoryConnectionDto; connections:ConnectionView[]; certificates:CertificateView[]; keys:Record<string,string>;
  } = $props();
  let selectedId=$state("");
  let editVersion=$state("");
  let open=$state(false);
  let busy=$state(false);
  let certificateId=$state("");
  let registryId=$state("");
  const selected=$derived(profiles.find(profile=>profile.id===selectedId));
  const selectedConnection=$derived(connections.find(connection=>connection.id===registryId));
  const imageDestination=$derived(selectedConnection ? registryImageRepository(repository.provider.owner, repository.provider.name) : "");
  const failureMessage=(message:string)=>message==="runtime_verification_failed" ? "Runtime image verification failed. Rebuild explicitly to replace this image." : /^[a-z][a-z0-9_]+$/u.test(message) ? words(message) : message;
  const selection=(profile:RuntimeProfileDto)=>profile.registry ?? {registryConnectionId:repository.registryConnectionId ?? ""};
  function edit(profile:RuntimeProfileDto) {
    selectedId=profile.id;
    editVersion=profile.resourceVersion;
    certificateId=profile.certificateId ?? "";
    registryId=selection(profile).registryConnectionId;
    open=true;
  }
  const submit:SubmitFunction=({action})=>{
    const registrySave=(action.searchParams.has("/registry") || action.searchParams.has("/certificate"));
    busy=true;
    return async({result,update})=>{
      busy=false;
      await update({reset:false});
      await tick();
      if(result.type==="success" && registrySave) editVersion=selected?.resourceVersion ?? "";
    };
  };
  let previous: Map<string, string> | undefined;
  $effect(()=>{
    for (const profile of profiles) {
      if (previous?.has(profile.id) && previous.get(profile.id) !== profile.state) {
        notify(["failed", "degraded"].includes(profile.state) ? "warning" : "info", "Runtime update", `${words(profile.kind)} runtime is ${words(profile.state).toLowerCase()}.`);
      }
    }
    previous=new Map(profiles.map(profile=>[profile.id,profile.state]));
  });
</script>

<section class="space-y-4">
  <div class="flex flex-wrap items-center justify-between gap-2"><h2 class="text-lg font-semibold">Runtimes</h2><PollStatus /></div>
  {#each profiles as profile (profile.id)}
    {@const revision=revisions[profile.id]}
    <InformationCard title={words(profile.kind)} navigationHref={`/_old/profiles/${encodeURIComponent(profile.id)}`} status={profile.state} actions={[{ label: "Edit", onSelect: () => edit(profile), disabled: busy }]}>
      {#if profile.state === "updating"}<p class="text-sm text-muted-foreground" role="status">The environment needs an update. New tasks wait until preparation and verification complete.</p>{/if}
      {#each profile.findings as finding (finding.code)}<div class="text-sm" role="status"><p class="text-destructive">{finding.message}</p><p class="text-muted-foreground">{finding.remediation}</p></div>{/each}
      <InformationTable rows={[
        ...(profile.preparationPhase ? [{ label: "Preparation", value: words(profile.preparationPhase) }] : []),
        ...(profile.kind === "ayni_managed" ? [
          { label: "Ayni lock", value: revision?.managed?.fingerprint || "Not available yet", monospace: true },
          { label: "Certificate", value: profile.effectiveCertificate ? `${profile.effectiveCertificate.displayName} · revision ${profile.effectiveCertificate.revision}${profile.certificateId ? "" : " (default)"}` : "Not configured" },
        ] : []),
        ...profile.reconciliation.filter(entry => entry.key === "discovery-reference").map(entry => ({ label: "Discovery tag", value: entry.value, monospace: true })),
        { label: "Image destination", value: profile.desiredImageRepository || revision?.managed?.repository || "Not configured" },
      ]} />
      {#if profile.kind==="ayni_managed" && !profile.effectiveCertificate}<p class="text-sm text-muted-foreground"><a href="/settings/ayni/certificates" class="underline">Add a signing certificate</a> to build this environment.</p>{/if}
      {#if profile.failure && !profile.findings.length}<p class="text-sm text-destructive" role="status">{failureMessage(profile.failure.message)}</p>{/if}
      {#if profile.support!=="operational"}<p class="text-sm text-muted-foreground">{words(profile.support)} runtime</p>{/if}
    </InformationCard>
  {:else}<EmptyState title={repository.useAyni === false ? "No runtimes configured" : "Inspection pending"} description={repository.useAyni === false ? "Enable Ayni when you want to prepare a managed runtime." : "Runtime status will appear here when inspection completes."} />{/each}
</section>

<Dialog.Root bind:open>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header><Dialog.Title>Edit runtime</Dialog.Title><Dialog.Description>{selected ? `${words(selected.kind)} · ${repository.displayName}` : "This runtime is no longer available."}</Dialog.Description></Dialog.Header>

    {#if selected}
      <StatusBadge status={selected.state} tone={statusTone(selected.state)} />
      {#if selected.failure}<Feedback kind="warning" title="Runtime needs attention" message={failureMessage(selected.failure.message)} />{/if}
      {#if selected.kind==="ayni_managed"}
      <form method="POST" action="?/certificate" use:enhance={submit} class="space-y-4">
        <input type="hidden" name="profileId" value={selected.id}/><input type="hidden" name="idempotencyKey" value={`${keys[selected.id]}-certificate-${editVersion}`}/><input type="hidden" name="resourceVersion" value={editVersion}/>
        <div class="space-y-2"><Label for="runtime-certificate">Signing certificate</Label><ThemedSelect id="runtime-certificate" name="certificateId" bind:value={certificateId} options={[{value:"",label:certificates.find(c=>c.isDefault) ? `Global default (${certificates.find(c=>c.isDefault)!.displayName})` : "Global default (not configured)"},...certificates.map(c=>({value:c.id,label:c.displayName}))]} /></div>
        {#if selected.effectiveCertificate}<p class="text-xs text-muted-foreground">Effective key: {selected.effectiveCertificate.keyId} · revision {selected.effectiveCertificate.revision}</p>{/if}
        {#if !certificates.length}<p class="text-sm text-muted-foreground"><a href="/settings/ayni/certificates" class="underline">Add a signing certificate</a> to build this environment.</p>{/if}
        <Button type="submit" disabled={busy || !certificates.length}>Save certificate</Button>
      </form>
      <form method="POST" action="?/registry" use:enhance={submit} class="space-y-4">
        <input type="hidden" name="profileId" value={selected.id}/><input type="hidden" name="idempotencyKey" value={keys[selected.id]}/><input type="hidden" name="resourceVersion" value={editVersion}/>
        <div class="space-y-2"><Label for="runtime-registry">GHCR connection</Label><ThemedSelect id="runtime-registry" name="registryConnectionId" bind:value={registryId} options={connections.map(connection=>({value:connection.id,label:connection.displayName}))} required /></div>
        {#if imageDestination}<p class="break-all text-xs text-muted-foreground">Image destination: {imageDestination}</p>{/if}
        {#if !connections.length}<p class="text-sm text-muted-foreground">Add a ready <a href="/settings/container-registries/github" class="underline">GHCR connection</a> to configure this runtime.</p>{/if}
        <Button type="submit" disabled={busy || !registryId}>Save image destination</Button>
      </form>
      {/if}
      <div class="flex flex-wrap gap-2 border-t pt-4">
        {#each ["sync","rebuild"] as action (action)}<form method="POST" action={`?/${action}`} use:enhance={submit}><input type="hidden" name="profileId" value={selected.id}/><input type="hidden" name="idempotencyKey" value={keys[selected.id]}/><Button type="submit" variant="outline" disabled={busy || (action==="rebuild" && !selected.rebuildAvailable)}>{action==="sync" ? selected.failure ? "Retry" : "Build / Retry" : "Rebuild runtime"}</Button></form>{/each}
        <Button href={`/_old/profiles/${encodeURIComponent(selected.id)}`} variant="outline">View details</Button>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
