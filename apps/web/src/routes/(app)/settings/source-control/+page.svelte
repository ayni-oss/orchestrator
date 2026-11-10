<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import ProviderCard from "$lib/components/provider-card.svelte";
  const providers = [{name:'GitHub',logo:'github',description:'Connect your GitHub App and choose which repositories ayni can access.',href:'/settings/source-control/github'}];
</script>

<svelte:head><title>Source control · ayni Orchestrator</title></svelte:head>
<PageHeader title="Source control" />
<PageContent>
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <ProviderCard {...provider} />
    {/each}
  </div>
</PageContent>
