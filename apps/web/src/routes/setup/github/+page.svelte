<script lang="ts">
  import ConnectionManager from "$lib/components/connection-manager.svelte";
  import type { PageProps } from "./$types";
  let { data, form }:PageProps = $props();
  import { onMount } from "svelte";
  import { ExternalLink, KeyRound, ShieldCheck, Upload } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";

  let owner = $state("");
  const ownerIsValid = $derived(/^[A-Za-z0-9](?:[A-Za-z0-9-]{0,98}[A-Za-z0-9])?$/u.test(owner));
  let flowState = $state("");
  let callbackOrigin = $state("");
  let appSuffix = $state("");
  const organizationAppUrl = $derived(ownerIsValid && flowState ? `https://github.com/organizations/${encodeURIComponent(owner)}/settings/apps/new?state=${encodeURIComponent(flowState)}` : undefined);
  const personalAppUrl = $derived(flowState ? `https://github.com/settings/apps/new?state=${encodeURIComponent(flowState)}` : undefined);
  const appName = $derived(`Ayni Orchestrator (${appSuffix})`);
  const appManifest = $derived(JSON.stringify({
    name: appName,
    description: "Customer-owned GitHub App for a trusted local Ayni Orchestrator deployment.",
    url: "https://github.com/ayni-oss/orchestrator",
    redirect_url: callbackOrigin ? `${callbackOrigin}/setup/github/complete` : undefined,
    public: false,
    default_permissions: {
      contents: "write",
      metadata: "read",
      pull_requests: "write",
      workflows: "write",
    },
  }));
  onMount(async () => {
    callbackOrigin = window.location.origin;
    appSuffix = Array.from(crypto.getRandomValues(new Uint8Array(4)), byte => byte.toString(16).padStart(2, "0")).join("");
    const response = await fetch("/setup/github/start", { method: "POST" });
    if (response.ok) flowState = (await response.json() as { state: string }).state;
  });
</script>

<svelte:head>
  <title>GitHub source · Ayni Orchestrator</title>
  <meta name="description" content="Create or configure a customer-owned GitHub App for Ayni Orchestrator." />
</svelte:head>

<div class="space-y-8">
  <div class="max-w-3xl space-y-3">
    <p class="text-sm font-medium text-primary">GitHub integration</p>
    <h1 class="text-3xl font-semibold tracking-tight">GitHub</h1>
    <p class="text-muted-foreground">GitHub Apps and their credentials remain under your account or organization. Ayni stores credentials in Kubernetes Secrets. Validated replacements activate automatically.</p>
  </div>

  <ol class="grid gap-3 text-sm sm:grid-cols-4" aria-label="GitHub onboarding steps">
    <li class="rounded-lg border bg-card p-4"><strong class="block">1. Configure App</strong><span class="text-muted-foreground">Create or select an App.</span></li>
    <li class="rounded-lg border bg-card p-4"><strong class="block">2. Store credentials</strong><span class="text-muted-foreground">Keep the private key in your secret store.</span></li>
    <li class="rounded-lg border bg-card p-4"><strong class="block">3. Install App</strong><span class="text-muted-foreground">Grant selected repository access in GitHub.</span></li>
    <li class="rounded-lg border bg-card p-4"><strong class="block">4. Enroll repository</strong><span class="text-muted-foreground">Return here to make it ready.</span></li>
  </ol>

  <div class="grid gap-6">{#if data.available && data.connections.length===0}
    <Card.Root id="create">
      <Card.Header>
        <div class="flex items-center gap-3"><span class="grid size-9 place-items-center rounded-md bg-accent"><ShieldCheck class="size-4" aria-hidden="true" /></span><div><Card.Title>Create a new GitHub App</Card.Title><Card.Description>Create the App under the customer organization so its ownership and credentials stay with them.</Card.Description></div></div>
      </Card.Header>
      <Card.Content class="space-y-5 text-sm">
        <div class="space-y-2">
          <Label for="github-owner">GitHub organization</Label>
          <Input id="github-owner" bind:value={owner} autocomplete="organization" placeholder="acme-inc" aria-describedby="github-owner-help" />
          <p id="github-owner-help" class="text-xs text-muted-foreground">This opens GitHub’s organization-specific App creation page, avoiding an ambiguous owner picker.</p>
        </div>
        <div class="rounded-md bg-muted p-4">
          <p class="font-medium">Configure these App settings in GitHub</p>
          <ul class="mt-2 list-disc space-y-1 pl-5 text-muted-foreground">
            <li>App name: <strong class="text-foreground">{appName}</strong>. The random suffix makes the name unique while keeping it within GitHub’s 34-character limit.</li>
            <li>Repository permissions: <strong class="text-foreground">Metadata: Read-only</strong>; <strong class="text-foreground">Contents, Pull requests, and Workflows: Read and write</strong>.</li>
            <li>Install only on the repositories Ayni should operate.</li>
            <li>Generate a private key after creating the App. GitHub displays its download once—store it before leaving the page.</li>
          </ul>
        </div>
        {#if organizationAppUrl}
          <form action={organizationAppUrl} method="POST" target="_blank">
            <input type="hidden" name="manifest" value={appManifest} />
            <Button type="submit">Create App for {owner} <ExternalLink aria-hidden="true" /></Button>
          </form>
        {:else}
          <Button disabled>Enter an organization name to continue</Button>
        {/if}
        <p class="text-xs text-muted-foreground">This uses GitHub’s App Manifest flow with Ayni’s required permissions. For a personal account, use the same pre-populated flow:</p>
        {#if personalAppUrl}
          <form action={personalAppUrl} method="POST" target="_blank">
            <input type="hidden" name="manifest" value={appManifest} />
            <Button type="submit" variant="outline">Create App for personal account <ExternalLink aria-hidden="true" /></Button>
          </form>
        {:else}
          <Button disabled variant="outline">Preparing GitHub setup…</Button>
        {/if}
      </Card.Content>
    </Card.Root>

  {/if}</div>
  {#if form?.error || data.error}<p role="alert">{(form?.error ?? data.error).message}</p>{/if}
  {#if form?.success}<p role="status">{form.success}</p>{/if}
  <ConnectionManager {...data} kind="github_app"/>
  <Button href="/repositories#enroll">Enroll a repository</Button>
</div>
