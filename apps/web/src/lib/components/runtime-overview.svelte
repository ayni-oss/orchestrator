<script lang="ts">
  import { onMount, tick } from "svelte";
  import { enhance } from "$app/forms";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { PageData } from "../../routes/repositories/[repositoryId]/$types";
  type RuntimeProfileDto = PageData["runtimeProfiles"][number];
  type RepositoryConnectionDto = NonNullable<PageData["repositoryConnection"]>;
  import type { ConnectionView } from "$lib/connection.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import ThemedSelect from "$lib/components/ui/select/select-field.svelte";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import StatusBadge from "$lib/components/status-badge.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
  import PollStatus from "$lib/components/poll-status.svelte";
  import { statusTone, words } from "$lib/presentation.js";

  type Revision = { managed?: {fingerprint:string;repository:string} | undefined };
  let { profiles, revisions, repository, connections, keys, error }: {
    profiles:RuntimeProfileDto[]; revisions:Record<string,Revision | undefined>;
    repository:RepositoryConnectionDto; connections:ConnectionView[]; keys:Record<string,string>; error?:string | undefined;
  } = $props();
  let selectedId=$state("");
  let editVersion=$state("");
  let open=$state(false);
  let busy=$state(false);
  let showError=$state(false);
  let registryId=$state("");
  let namespace=$state("");
  let notification=$state("");
  let notificationKind=$state<"success"|"warning">("success");
  const selected=$derived(profiles.find(profile=>profile.id===selectedId));
  const selectedConnection=$derived(connections.find(connection=>connection.id===registryId));
  const failureMessage=(message:string)=>message==="runtime_verification_failed" ? "Runtime image verification failed. Sync or rebuild to retry." : /^[a-z][a-z0-9_]+$/u.test(message) ? words(message) : message;
  const selection=(profile:RuntimeProfileDto)=>profile.registry ?? {registryConnectionId:repository.registryConnectionId ?? "",imageNamespace:repository.imageNamespace ?? ""};
  function edit(profile:RuntimeProfileDto) {
    selectedId=profile.id;
    editVersion=profile.resourceVersion;
    registryId=selection(profile).registryConnectionId;
    namespace=selection(profile).imageNamespace;
    showError=false;
    open=true;
  }
  const submit:SubmitFunction=({action})=>{
    const registrySave=action.searchParams.has("/registry");
    busy=true;showError=false;
    return async({result,update})=>{
      busy=false;showError=result.type==="failure";
      if(result.type==="success") { notificationKind="success"; notification=typeof result.data?.success==="string" ? result.data.success : "Runtime action requested."; }
      await update({reset:false});
      await tick();
      if(result.type==="success" && registrySave) editVersion=selected?.resourceVersion ?? "";
    };
  };
  onMount(()=>{
    let previous=new Map(profiles.map(profile=>[profile.id,profile.state]));
    const timer=setInterval(()=>{
      const changed=profiles.find(profile=>previous.get(profile.id)!==profile.state);
      if(changed) { notificationKind=["failed","degraded"].includes(changed.state) ? "warning" : "success"; notification=`${words(changed.kind)} runtime is ${words(changed.state).toLowerCase()}.`; }
      previous=new Map(profiles.map(profile=>[profile.id,profile.state]));
    },1000);
    return ()=>clearInterval(timer);
  });
</script>

{#if notification}<div class="flex items-start gap-2"><div class="flex-1"><Feedback kind={notificationKind} title="Runtime update" message={notification} /></div><Button variant="ghost" size="sm" aria-label="Dismiss notification" onclick={()=>notification=""}>Dismiss</Button></div>{/if}
<section class="space-y-4">
  <div class="flex flex-wrap items-center justify-between gap-2"><h2 class="text-lg font-semibold">Runtimes</h2><PollStatus /></div>
  {#each profiles as profile (profile.id)}
    {@const runtimeRegistry=selection(profile)}
    {@const revision=revisions[profile.id]}
    <article class="space-y-3 rounded-xl border bg-card p-4">
      <div class="flex items-center justify-between gap-3"><a class="font-semibold hover:underline" href={`/profiles/${encodeURIComponent(profile.id)}`}>{words(profile.kind)}</a><div class="flex items-center gap-2"><StatusBadge status={profile.state} tone={statusTone(profile.state)} /><Button variant="outline" size="sm" onclick={()=>edit(profile)}>{profile.support==="operational" ? "Edit" : "Details"}</Button></div></div>
      <dl class="grid gap-2 text-sm sm:grid-cols-[7rem_1fr]">
        {#if profile.kind==="ayni_managed"}<dt class="text-muted-foreground">Ayni lock</dt><dd class="break-all font-mono text-xs">{revision?.managed?.fingerprint ?? "Not available yet"}</dd>{/if}
        <dt class="text-muted-foreground">Image destination</dt><dd class="break-all">{runtimeRegistry.registryConnectionId ? `ghcr.io/${runtimeRegistry.imageNamespace}/${repository.provider.owner.toLowerCase()}/${repository.provider.name.toLowerCase()}-ayni` : revision?.managed?.repository ?? "Not configured"}</dd>
      </dl>
      {#if profile.failure}<p class="text-sm text-destructive" role="status">{failureMessage(profile.failure.message)}</p>{/if}
      {#if profile.support!=="operational"}<p class="text-sm text-muted-foreground">{words(profile.support)} runtime</p>{/if}
    </article>
  {:else}<EmptyState title="Inspection pending" description="Runtime status will appear here when inspection completes." />{/each}
</section>

<Dialog.Root bind:open>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header><Dialog.Title>Edit runtime</Dialog.Title><Dialog.Description>{selected ? `${words(selected.kind)} · ${repository.displayName}` : "This runtime is no longer available."}</Dialog.Description></Dialog.Header>
    {#if showError && error}<Feedback kind="error" title="Runtime action failed" message={error} />{/if}
    {#if selected}
      <StatusBadge status={selected.state} tone={statusTone(selected.state)} />
      {#if selected.failure}<Feedback kind="warning" title="Runtime needs attention" message={failureMessage(selected.failure.message)} />{/if}
      {#if selected.kind==="ayni_managed"}
      <form method="POST" action="?/registry" use:enhance={submit} class="space-y-4">
        <input type="hidden" name="profileId" value={selected.id}/><input type="hidden" name="idempotencyKey" value={keys[selected.id]}/><input type="hidden" name="resourceVersion" value={editVersion}/>
        <div class="space-y-2"><Label for="runtime-registry">GHCR connection</Label><ThemedSelect id="runtime-registry" name="registryConnectionId" bind:value={registryId} options={connections.map(connection=>({value:connection.id,label:connection.displayName}))} required onValueChange={value=>{registryId=value;namespace=connections.find(connection=>connection.id===value)?.imageNamespace ?? "";}} /></div>
        <div class="space-y-2"><Label for="runtime-namespace">Image namespace</Label><Input id="runtime-namespace" name="imageNamespace" bind:value={namespace} required /></div>
        {#if selectedConnection}<p class="text-xs text-muted-foreground">Approved namespace: {selectedConnection.imageNamespace}</p>{/if}
        {#if !connections.length}<p class="text-sm text-muted-foreground">Add a ready <a href="/container-registries/github" class="underline">GHCR connection</a> to configure this runtime.</p>{/if}
        <Button type="submit" disabled={busy || !registryId}>Save image destination</Button>
      </form>
      {/if}
      <div class="flex flex-wrap gap-2 border-t pt-4">
        {#each ["sync","rebuild"] as action (action)}<form method="POST" action={`?/${action}`} use:enhance={submit}><input type="hidden" name="profileId" value={selected.id}/><input type="hidden" name="idempotencyKey" value={keys[selected.id]}/><Button type="submit" variant="outline" disabled={busy}>{action==="sync" ? "Sync runtime" : "Rebuild runtime"}</Button></form>{/each}
        <Button href={`/profiles/${encodeURIComponent(selected.id)}`} variant="outline">View details</Button>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
