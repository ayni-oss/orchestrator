<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import ProviderCard from "$lib/components/provider-card.svelte";
  const providers = [{name:'OpenAI',logo:'openai',description:'Manage API connections for project conversations.',href:'/settings/llm-providers/openai'}];
</script>

<svelte:head><title>LLM providers · Ayni Orchestrator</title></svelte:head>
<PageHeader title="LLM providers" />
<PageContent>
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <ProviderCard {...provider} />
    {/each}
  </div>
</PageContent>
