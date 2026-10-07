<script lang="ts">
  import { onMount } from "svelte";
  import { enhance } from "$app/forms";
  import type { SubmitFunction } from "@sveltejs/kit";
  import { invalidateAll } from "$app/navigation";
  import { Button } from "$lib/components/ui/button/index.js";
  import type { ConnectionView } from "$lib/connection.js";
  let {connections,idempotencyKey,kind,available=true}:{connections:ConnectionView[];idempotencyKey:string;kind:string;available?:boolean}=$props();
  onMount(()=>{const timer=setInterval(()=>{void invalidateAll();},30_000);return ()=>clearInterval(timer);});
  const github=$derived(kind==="github_app");
  // Every response clears secret inputs, including rejected submissions.
  const submit:SubmitFunction=({formElement})=>async ({update})=>{formElement.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>('input[type=password],textarea').forEach(e=>e.value="");await update();await invalidateAll();};
</script>
<div class="space-y-6">
  {#if !available}<p role="status">Connection service unavailable. Retry when the control plane is ready.</p>{/if}
  {#each connections as c (c.id)}
    <section class="space-y-4 rounded-xl border bg-card p-6">
      <h2 class="text-lg font-semibold">{c.displayName}</h2>
      <p class="text-sm text-muted-foreground">{c.status} · {github ? `App ${c.appId}` : `ghcr.io/${c.imageNamespace}`}</p>
      {#if kind==="ghcr"}<p class="text-sm">Authentication: {c.testAuthentication || "pending"} · Push: {c.testPush || "pending"} · Pull: {c.testPull || "pending"} · Cleanup: {c.testCleanup || "pending"}</p>{/if}
      {#if c.failureReason}<p role="status">{c.failureReason}</p>{/if}
      {#if c.testCleanupFailureReason}<p role="status">Artifact cleanup: {c.testCleanupFailureReason}. Retry after restoring package deletion permission.</p>{/if}
      {#if c.managementSource==="admin"}
        <p>Managed by your administrator. Update credentials in your deployment’s secret store.</p>
      {:else}
        <form method="POST" action="?/replace" use:enhance={submit} class="space-y-3">
          <input type="hidden" name="id" value={c.id}/><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-replace-${c.id}`}/><input type="hidden" name="resourceVersion" value={c.resourceVersion}/>
          {#if github}<label class="block">New private key<textarea name="privateKey" required autocomplete="off" class="mt-1 block w-full rounded border bg-background p-2"></textarea></label>
          {:else}<label class="block">New push token<input name="token" type="password" required autocomplete="new-password" class="mt-1 block w-full rounded border bg-background p-2"/></label><label class="block">Separate pull-only token<input name="pullToken" type="password" required autocomplete="new-password" class="mt-1 block w-full rounded border bg-background p-2"/></label>{/if}
          {#if !github}<p class="text-sm text-muted-foreground">Use separate classic personal access tokens. The pull token requires read:packages only; write:packages and delete:packages are rejected. The push token requires write:packages; add delete:packages to clean up test artifacts. Both tokens must have access to the selected private namespace. Ayni does not grant GitHub permissions.</p>{/if}
          <Button type="submit">Replace credentials</Button>
        </form>
        <div class="flex flex-wrap gap-3">
          {#each ["test","toggle","delete"] as action}
            <form method="POST" action={`?/${action}`} use:enhance={submit}>
              <input type="hidden" name="id" value={c.id}/><input type="hidden" name="idempotencyKey" value={`${idempotencyKey}-${action}-${c.id}`}/><input type="hidden" name="resourceVersion" value={c.resourceVersion}/><input type="hidden" name="enabled" value={String(!c.enabled)}/>
              <Button type="submit" variant="outline">{action==="test" ? "Test connection / Retry" : action==="toggle" ? c.enabled ? "Disable" : "Enable" : "Delete"}</Button>
            </form>
          {/each}
        </div>
      {/if}
    </section>
  {/each}
  {#if available && (!github || connections.length===0)}
    <form method="POST" action="?/create" use:enhance={submit} class="space-y-4 rounded-xl border bg-card p-6">
      <h2 class="text-lg font-semibold">{github ? "Import an existing GitHub App" : "Add GHCR connection"}</h2>
      <input type="hidden" name="idempotencyKey" value={idempotencyKey}/>
      <label class="block">Connection name<input name="displayName" required maxlength="256" class="mt-1 block w-full rounded border bg-background p-2"/></label>
      {#if github}
        <label class="block">App ID<input name="appId" required inputmode="numeric" pattern="[0-9]+" class="mt-1 block w-full rounded border bg-background p-2"/></label>
        <label class="block">Private key (PEM)<textarea name="privateKey" required autocomplete="off" class="mt-1 block w-full rounded border bg-background p-2"></textarea></label>
      {:else}
        <label class="block">GitHub username<input name="username" required class="mt-1 block w-full rounded border bg-background p-2"/></label>
        <label class="block">Image namespace<input name="imageNamespace" required placeholder="organization" class="mt-1 block w-full rounded border bg-background p-2"/></label>
        <label class="block">Push token<input name="token" type="password" required autocomplete="new-password" class="mt-1 block w-full rounded border bg-background p-2"/></label>
        <label class="block">Separate pull-only token<input name="pullToken" type="password" required autocomplete="new-password" class="mt-1 block w-full rounded border bg-background p-2"/></label>
      {/if}
      {#if !github}<p class="text-sm text-muted-foreground">Use separate classic personal access tokens. The pull token requires read:packages only; write:packages and delete:packages are rejected. The push token requires write:packages; add delete:packages to clean up test artifacts. Both tokens must have access to the selected private namespace. Ayni does not grant GitHub permissions.</p>{/if}
      <p class="text-sm text-muted-foreground">Credentials are stored in Kubernetes Secrets and are never returned by this page. Validated replacements activate automatically.</p>
      <Button type="submit">Save and test</Button>
    </form>
  {/if}
</div>
