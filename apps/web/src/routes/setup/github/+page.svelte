<script lang="ts">
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
  const organizationAppUrl = $derived(ownerIsValid && flowState ? `https://github.com/organizations/${encodeURIComponent(owner)}/settings/apps/new?state=${encodeURIComponent(flowState)}` : undefined);
  const personalAppUrl = $derived(flowState ? `https://github.com/settings/apps/new?state=${encodeURIComponent(flowState)}` : undefined);
  const appManifest = $derived(JSON.stringify({
    name: "Ayni Orchestrator",
    description: "Customer-owned GitHub App for a trusted local Ayni Orchestrator deployment.",
    url: "https://github.com/ayni-oss/orchestrator",
    redirect_url: callbackOrigin ? `${callbackOrigin}/setup/github/complete` : undefined,
    public: false,
    default_permissions: {
      actions: "read",
      checks: "read",
      contents: "write",
      issues: "write",
      metadata: "read",
      packages: "read",
      pull_requests: "write",
      workflows: "write",
    },
  }));
  onMount(async () => {
    callbackOrigin = window.location.origin;
    const response = await fetch("/setup/github/start", { method: "POST" });
    if (response.ok) flowState = (await response.json() as { state: string }).state;
  });
</script>

<svelte:head>
  <title>Connect GitHub · Ayni Orchestrator</title>
  <meta name="description" content="Create or configure a customer-owned GitHub App for Ayni Orchestrator." />
</svelte:head>

<div class="space-y-8">
  <div class="max-w-3xl space-y-3">
    <p class="text-sm font-medium text-primary">GitHub setup</p>
    <h1 class="text-3xl font-semibold tracking-tight">Connect a customer-owned GitHub App</h1>
    <p class="text-muted-foreground">GitHub Apps and their credentials remain under your account or organization. Ayni reads the App ID and private key only from the local deployment secret; neither is returned by the control plane.</p>
  </div>

  <ol class="grid gap-3 text-sm sm:grid-cols-4" aria-label="GitHub onboarding steps">
    <li class="rounded-lg border bg-card p-4"><strong class="block">1. Configure App</strong><span class="text-muted-foreground">Create or select an App.</span></li>
    <li class="rounded-lg border bg-card p-4"><strong class="block">2. Store credentials</strong><span class="text-muted-foreground">Keep the private key in your secret store.</span></li>
    <li class="rounded-lg border bg-card p-4"><strong class="block">3. Install App</strong><span class="text-muted-foreground">Grant selected repository access in GitHub.</span></li>
    <li class="rounded-lg border bg-card p-4"><strong class="block">4. Enroll repository</strong><span class="text-muted-foreground">Return here to make it ready.</span></li>
  </ol>

  <div class="grid gap-6 xl:grid-cols-2">
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
            <li>Repository permissions: <strong class="text-foreground">Actions, Checks, and Packages: Read-only</strong>; <strong class="text-foreground">Contents, Issues, Pull requests, and Workflows: Read and write</strong>.</li>
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

    <Card.Root id="bring-your-own">
      <Card.Header>
        <div class="flex items-center gap-3"><span class="grid size-9 place-items-center rounded-md bg-accent"><Upload class="size-4" aria-hidden="true" /></span><div><Card.Title>Bring an existing GitHub App</Card.Title><Card.Description>Use an organization-approved App without changing its ownership or credentials.</Card.Description></div></div>
      </Card.Header>
      <Card.Content class="space-y-4 text-sm">
        <ol class="list-decimal space-y-2 pl-5 text-muted-foreground">
          <li>Confirm the App has the exact permissions listed for the creation flow, including read-only Actions, Checks, and Packages access.</li>
          <li>Generate or obtain an App private key from the App owner.</li>
          <li>Install the App on the repositories this local control plane may access.</li>
          <li>Use the App ID and private key in the deployment secret configuration below.</li>
        </ol>
        <div class="rounded-md border border-amber-500/30 bg-amber-500/10 p-4 text-muted-foreground"><strong class="text-foreground">Repository access is managed by GitHub.</strong> If a repository is missing later, update the App installation in GitHub, then return to Repositories and enroll it. Ayni does not expand GitHub installation scope automatically.</div>
      </Card.Content>
    </Card.Root>
  </div>

  <Card.Root>
    <Card.Header><div class="flex items-center gap-3"><span class="grid size-9 place-items-center rounded-md bg-accent"><KeyRound class="size-4" aria-hidden="true" /></span><div><Card.Title>Store credentials and continue</Card.Title><Card.Description>Use your deployment’s secret store. Do not paste private keys into this browser.</Card.Description></div></div></Card.Header>
    <Card.Content class="space-y-4 text-sm">
      <p>For the supported local profile, put the App ID and base64-encoded downloaded PEM in <code>dev/.env</code>, keep that file mode <code>0600</code>, then run:</p>
      <pre class="overflow-x-auto rounded-md bg-muted p-4 text-xs"><code>dev/ctl secrets
dev/ctl deploy</code></pre>
      <p class="text-muted-foreground">The deployment creates the Kubernetes Secret from your local file. The web UI and gRPC API never expose the private key. After deployment, install the App in GitHub, record its installation ID, and enroll a repository.</p>
      <Button href="/repositories#enroll">Enroll a repository</Button>
    </Card.Content>
  </Card.Root>
</div>
