<script lang="ts">
  import { notify } from "$lib/notifications.js";
  import { onMount } from "svelte";
  import { enhance } from "$lib/forms.js";
  import { refreshScope, subscribeRefresh } from "$lib/refresh.js";
  import type { SubmitFunction } from "@sveltejs/kit";
  import type { CertificateView } from "$lib/certificate.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  let {certificates,idempotencyKey,available=true}: {certificates:CertificateView[];idempotencyKey:string;available?:boolean}=$props();
  let createOpen=$state(false),editOpen=$state(false),selectedId=$state("");
  let createSeed=$state(""),editSeed=$state(""),createMode=$state("generate"),editMode=$state("generate");
  let busy=$state(false);
  const selected=$derived(certificates.find(c=>c.id===selectedId));
  function clearSeeds() { createSeed="";editSeed=""; }
  function closeCreate(open:boolean) {createOpen=open;if(!open){clearSeeds();}}
  function closeEdit(open:boolean) {editOpen=open;if(!open){clearSeeds();}}
  function edit(id:string) {selectedId=id;clearSeeds();editMode="generate";editOpen=true;}
  function submit(operation:string):SubmitFunction {
    return ({formData})=>{
      busy=true;clearSeeds();
      // FormData already contains the submitted seed; remove it after the request completes.
      return async({result,update})=>{
        formData.delete("signingSeed");clearSeeds();
        try {await update({reset:false,invalidateAll:false});if(result.type==="success" && operation==="create")closeCreate(false);if(result.type==="success" && operation==="delete")closeEdit(false);await refreshScope("app:certificates", true);}
        finally {busy=false;clearSeeds();}
      };
    };
  }
  const snippet=(c:CertificateView)=>`[environment.certificate.trusted_keys]\n${JSON.stringify(c.keyId)} = ${JSON.stringify(c.publicKey)}`;
  async function copy(value:string) {try{await navigator.clipboard.writeText(value);notify("success", "Copied to clipboard");}catch{notify("error", "Copy unavailable", "Select and copy the displayed value.");}}
  onMount(()=>{const unsubscribe=subscribeRefresh("app:certificates",{intervalMs:30_000});return()=>{unsubscribe();clearSeeds();};});
</script>

{#snippet identityFields(prefix:string)}
  <div class="space-y-2"><Label for={`${prefix}-key-id`}>Key ID</Label><Input id={`${prefix}-key-id`} name="keyId" required maxlength={128} pattern="[A-Za-z0-9_.\-]+" value={prefix==="replace" ? selected?.keyId : ""} /></div>
{/snippet}
{#snippet metadata(c:CertificateView,operation:string)}
  <input type="hidden" name="id" value={c.id}/><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-${operation}-${c.id}-${c.resourceVersion}`}/><input type="hidden" name="resourceVersion" value={c.resourceVersion}/>
{/snippet}
<div class="space-y-4">
  <div class="flex items-center justify-between gap-3"><h2 class="text-lg font-semibold">Signing certificates</h2><Button onclick={()=>{createMode="generate";clearSeeds();createOpen=true;}} disabled={!available}>Add certificate</Button></div>
  {#if !certificates.length}<p class="text-sm text-muted-foreground">Add a certificate to sign Ayni environment images. The first certificate becomes the default.</p>{/if}
  {#each certificates as c (c.id)}
    <InformationCard title={c.displayName} status={c.enabled ? c.status : "disabled"} actions={[{ label: c.managementSource === "admin" ? "Details" : "Edit", onSelect: () => edit(c.id) }]}>
      <InformationTable rows={[
        { label: "Key ID", value: c.keyId },
        { label: "Revision", value: String(c.revision) },
        { label: "Runtimes", value: c.usageCount },
        { label: "Default", value: c.isDefault ? "Yes" : "No" },
        ...(c.managementSource === "admin" ? [{ label: "Managed by", value: "Your administrator" }] : []),
      ]} />
      {#if c.failureReason}<Feedback kind="warning" title="Latest operation needs attention" message={c.failureReason} />{/if}
    </InformationCard>
  {/each}
</div>
<Dialog.Root open={createOpen} onOpenChange={closeCreate}>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl"><Dialog.Header><Dialog.Title>Add certificate</Dialog.Title><Dialog.Description>Generate an Ed25519 key or import an existing Ayni signing seed.</Dialog.Description></Dialog.Header>

    <form method="POST" action="?/create" use:enhance={submit("create")} class="space-y-4">
      <input type="hidden" name="idempotencyKey" value={idempotencyKey}/>
      <div class="space-y-2"><Label for="certificate-name">Certificate name</Label><Input id="certificate-name" name="displayName" required maxlength={128}/></div>
      {@render identityFields("create")}
      <fieldset class="space-y-2"><legend class="text-sm font-medium">Signing key</legend><label class="mr-4 inline-flex items-center gap-2"><input type="radio" name="mode" value="generate" bind:group={createMode} onchange={clearSeeds}/>Generate</label><label class="inline-flex items-center gap-2"><input type="radio" name="mode" value="import" bind:group={createMode} onchange={clearSeeds}/>Import</label></fieldset>
      {#if createMode==="import"}<div class="space-y-2"><Label for="certificate-seed">Signing seed</Label><Input id="certificate-seed" name="signingSeed" type="password" required minlength={64} maxlength={64} pattern={"[0-9a-f]{64}"} autocomplete="new-password" bind:value={createSeed}/><p class="text-xs text-muted-foreground">64 lowercase hexadecimal characters.</p></div>{/if}
      <p class="text-sm text-muted-foreground">Private seeds are stored in Kubernetes Secrets and never returned. You can copy the public trust policy after saving.</p>
      <div class="flex justify-end gap-2"><Button variant="outline" onclick={()=>closeCreate(false)}>Cancel</Button><Button type="submit" disabled={busy}>Save certificate</Button></div>
    </form>
  </Dialog.Content>
</Dialog.Root>
<Dialog.Root open={editOpen} onOpenChange={closeEdit}>
  <Dialog.Content class="w-max max-w-[calc(100vw-2rem)] max-h-[90vh] overflow-y-auto sm:max-w-[calc(100vw-2rem)] [&>*]:min-w-0"><Dialog.Header><Dialog.Title>{selected?.managementSource==="admin" ? "Certificate details" : "Edit certificate"}</Dialog.Title><Dialog.Description>{selected?.displayName ?? "This certificate is no longer available."}</Dialog.Description></Dialog.Header>

    {#if selected}
      <dl class="space-y-2 text-sm"><div><dt class="text-muted-foreground">Public key</dt><dd class="break-all font-mono text-xs">{selected.publicKey}</dd></div><div><dt class="text-muted-foreground">Key ID / revision</dt><dd>{selected.keyId} / {selected.revision}</dd></div></dl>
      <div class="space-y-2"><p class="text-sm font-medium">Public trust policy</p><pre class="overflow-x-auto rounded-lg bg-muted p-3 text-xs">{snippet(selected)}</pre><div class="flex flex-wrap gap-2"><Button variant="outline" onclick={()=>copy(selected!.publicKey)}>Copy public key</Button><Button variant="outline" onclick={()=>copy(snippet(selected!))}>Copy trust policy</Button></div></div>
      {#if selected.failureReason}<Feedback kind="warning" title="Certificate needs attention" message={selected.failureReason}/>{/if}
      {#if selected.managementSource==="admin"}<p class="text-sm">Managed by your administrator. Update the configured Kubernetes Secret to rotate this certificate.</p><p class="text-xs text-muted-foreground">Source Secret: {selected.sourceSecretName || selected.secretName}</p>
      {:else}
        <form method="POST" action="?/rename" use:enhance={submit("rename")} class="space-y-2">{@render metadata(selected,"rename")}<Label for="certificate-edit-name">Certificate name</Label><Input id="certificate-edit-name" name="displayName" required maxlength={128} value={selected.displayName}/><Button type="submit" variant="outline" disabled={busy}>Save name</Button></form>
        <form method="POST" action="?/replace" use:enhance={submit("replace")} class="space-y-3 border-t pt-4">
          {@render metadata(selected,"replace")}{@render identityFields("replace")}
          <fieldset><legend class="mb-2 text-sm font-medium">Replace signing key</legend><label class="mr-4 inline-flex items-center gap-2"><input type="radio" name="mode" value="generate" bind:group={editMode} onchange={clearSeeds}/>Generate</label><label class="inline-flex items-center gap-2"><input type="radio" name="mode" value="import" bind:group={editMode} onchange={clearSeeds}/>Import</label></fieldset>
          {#if editMode==="import"}<div class="space-y-2"><Label for="certificate-edit-seed">New signing seed</Label><Input id="certificate-edit-seed" name="signingSeed" type="password" required minlength={64} maxlength={64} pattern={"[0-9a-f]{64}"} autocomplete="new-password" bind:value={editSeed}/></div>{/if}
          <p class="text-xs text-muted-foreground">Rotation proposes public trust updates for affected Ayni repositories. Existing tasks retain their frozen runtime.</p><Button type="submit" disabled={busy}>Replace signing key</Button>
        </form>
      {/if}
      <div class="flex flex-wrap gap-2 border-t pt-4">
        {#if !selected.isDefault && selected.enabled}<form method="POST" action="?/makeDefault" use:enhance={submit("default")}>{@render metadata(selected,"default")}<Button type="submit" variant="outline" disabled={busy}>Make default</Button></form>{/if}
        {#if selected.managementSource!=="admin"}
          <form method="POST" action="?/toggle" use:enhance={submit("toggle")}>{@render metadata(selected,"toggle")}<input type="hidden" name="enabled" value={String(!selected.enabled)}/><Button type="submit" variant="outline" disabled={busy || selected.isDefault}>{selected.enabled ? "Disable" : "Enable"}</Button></form>
          <form method="POST" action="?/delete" use:enhance={submit("delete")}>{@render metadata(selected,"delete")}<Button type="submit" variant="destructive" disabled={busy || selected.isDefault || BigInt(selected.usageCount)>0n}>Delete</Button></form>
        {/if}
      </div>
      {#if selected.isDefault}<p class="text-xs text-muted-foreground">Choose another default before disabling or deleting this certificate.</p>{:else if BigInt(selected.usageCount)>0n}<p class="text-xs text-muted-foreground">Assigned certificates cannot be deleted.</p>{/if}
    {/if}
  </Dialog.Content>
</Dialog.Root>
