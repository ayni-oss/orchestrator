<script lang="ts">
  import GithubConnectionManager from "$lib/components/github-connection-manager.svelte";
  import type { PageProps } from "./$types";
  let { data }: PageProps = $props();
</script>
<svelte:head><title>GitHub source · Ayni Orchestrator</title><meta name="description" content="Manage your GitHub App connection." /></svelte:head>
<div class="max-w-3xl space-y-8"><div class="max-w-3xl space-y-3"><h1 class="text-3xl font-semibold tracking-tight">GitHub</h1><p class="text-muted-foreground">Connect repositories using an App owned by your GitHub account or organization.</p></div>

<GithubConnectionManager {...data} />
</div>
