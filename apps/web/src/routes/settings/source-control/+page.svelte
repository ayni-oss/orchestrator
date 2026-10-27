<script lang="ts">
  import PageHeader from "$lib/components/page-header.svelte";
  import ProviderCard from "$lib/components/provider-card.svelte";
  const providers = [{name:'GitHub',logo:'github',description:'Connect your GitHub App and choose which repositories Ayni can access.',href:'/settings/source-control/github'}];
</script>

<svelte:head><title>Source control · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-5xl space-y-8">
  <PageHeader flush title="Source control" description="Choose a source provider for your repositories." />
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <ProviderCard {...provider} />
    {/each}
  </div>
</div>
