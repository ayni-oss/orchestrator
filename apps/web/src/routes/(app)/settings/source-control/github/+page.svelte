<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import GithubConnectionManager from "$lib/components/github-connection-manager.svelte";
</script>
<svelte:head><title>GitHub source · Ayni Orchestrator</title><meta name="description" content="Manage your GitHub App connection." /></svelte:head>
<PageHeader eyebrow="Source control" title="GitHub" />
<PageContent>

<GithubConnectionManager />
</PageContent>
