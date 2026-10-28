<script lang="ts">
  import PageHeader from "$lib/components/page-header.svelte";
  import ProviderCard from "$lib/components/provider-card.svelte";
  const providers = [{name:'GitHub',logo:'github',description:'Connect to GitHub Container Registry with separate push and pull access.',href:'/settings/container-registries/github'}];
</script>

<svelte:head><title>Container registries · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-5xl space-y-8">
  <PageHeader flush title="Container registries" description="Connect to GitHub Container Registry with separate push and pull access." />
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <ProviderCard {...provider} />
    {/each}
  </div>
</div>
