<script lang="ts">
  import { Download, ExternalLink, KeyRound } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import { Button } from "$lib/components/ui/button/index.js";
  import * as Card from "$lib/components/ui/card/index.js";

  let { data }: PageProps = $props();
  function downloadBundle() {
    const bundle = JSON.stringify({
      github_app_id: data.credentials.appId,
      github_client_id: data.credentials.clientId,
      github_client_secret: data.credentials.clientSecret,
      github_webhook_secret: data.credentials.webhookSecret,
      github_private_key_pem: data.credentials.privateKeyPem,
      github_app_url: data.credentials.appUrl,
    }, null, 2);
    const link = document.createElement("a");
    link.href = URL.createObjectURL(new Blob([bundle], { type: "application/json" }));
    link.download = `${data.credentials.slug}-github-app-credentials.json`;
    link.click();
    URL.revokeObjectURL(link.href);
  }
</script>

<svelte:head><title>Save GitHub App credentials · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-2xl space-y-6">
  <div><p class="text-sm font-medium text-primary">GitHub setup</p><h1 class="mt-2 text-3xl font-semibold tracking-tight">Save your App credentials</h1><p class="mt-2 text-muted-foreground">GitHub generated these credentials for your App. They are not stored by Ayni and this page is not cached.</p></div>
  <Card.Root><Card.Header><div class="flex items-center gap-3"><span class="grid size-9 place-items-center rounded-md bg-accent"><KeyRound class="size-4" /></span><div><Card.Title>Download once, then store safely</Card.Title><Card.Description>Store this bundle in your own secret manager or deployment configuration before continuing.</Card.Description></div></div></Card.Header><Card.Content class="space-y-4"><Button onclick={downloadBundle}><Download aria-hidden="true" /> Download credential bundle</Button><p class="text-sm text-muted-foreground">The bundle contains the App private key, client secret, and webhook secret. Treat it like a password and do not commit it.</p><Button href={data.credentials.appUrl} target="_blank" rel="noreferrer" variant="outline">Open GitHub App settings <ExternalLink aria-hidden="true" /></Button></Card.Content></Card.Root>
  <Card.Root><Card.Header><Card.Title>Next steps</Card.Title></Card.Header><Card.Content class="space-y-2 text-sm text-muted-foreground"><p>Install the App in GitHub and select the repositories it may access.</p><p>Then return to Ayni to enroll a selected repository. GitHub installation settings remain the place to add or remove repositories.</p><Button href="/repositories#enroll">Enroll a repository</Button></Card.Content></Card.Root>
</div>
