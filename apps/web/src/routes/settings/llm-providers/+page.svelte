<script lang="ts">
  import PageHeader from "$lib/components/page-header.svelte";
  import ProviderCard from "$lib/components/provider-card.svelte";
  const providers = [{name:'OpenAI',logo:'openai',description:'Manage API connections for workspace conversations.',href:'/settings/llm-providers/openai'}];
</script>

<svelte:head><title>LLM providers · Ayni Orchestrator</title></svelte:head>
<PageHeader title="LLM providers" />
<div class="mx-auto max-w-5xl space-y-8">
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <ProviderCard {...provider} />
    {/each}
  </div>
</div>
