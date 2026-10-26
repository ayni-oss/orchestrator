<script lang="ts">
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  const providers = [
    {name:"OpenAI",description:"Connect models for project research and coding tasks.",href:"/settings/llm-providers/openai"},
    {name:"Anthropic Claude",description:"Claude models from Anthropic.",href:null},
    {name:"AWS Bedrock",description:"Models hosted through Amazon Bedrock.",href:null},
  ];
</script>
<svelte:head><title>LLM providers · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-5xl space-y-8">
  <PageHeader flush title="LLM providers" description="Manage the models used to plan and implement project work." />
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <Card.Root><Card.Header><Card.Title>{provider.name}</Card.Title></Card.Header>
        <Card.Content class="flex-1"><p class="text-sm text-muted-foreground">{provider.description}</p></Card.Content>
        <Card.Footer>{#if provider.href}<Button href={provider.href}>Configure {provider.name}</Button>{:else}<Button variant="outline" disabled class="disabled:opacity-70">Not available</Button>{/if}</Card.Footer>
      </Card.Root>
    {/each}
  </div>
</div>
