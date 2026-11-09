<script lang="ts">
  import PageHeader from "$lib/components/page-header.svelte";
  import GithubConnectionManager from "$lib/components/github-connection-manager.svelte";
</script>
<svelte:head><title>GitHub source · Ayni Orchestrator</title><meta name="description" content="Manage your GitHub App connection." /></svelte:head>
<PageHeader eyebrow="Source control" title="GitHub" />
<div class="mx-auto max-w-5xl space-y-8">

<GithubConnectionManager />
</div>
