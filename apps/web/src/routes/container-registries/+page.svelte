<script lang="ts">
  import { Github, ArrowRight, Container } from "@lucide/svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
</script>

<svelte:head><title>Container registries · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader title="Container registries" description="Choose a registry destination for your testing environment images." />
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    <Card.Root>
      <Card.Header><Github class="mb-3 size-6" aria-hidden="true" /><Card.Title>GitHub</Card.Title><Card.Description>Store repository testing environments in GitHub Container Registry.</Card.Description></Card.Header>
      <Card.Footer><Button href="/container-registries/github">Configure GitHub <ArrowRight aria-hidden="true" /></Button></Card.Footer>
    </Card.Root>
    {#each ["Docker Hub", "Amazon ECR"] as provider (provider)}
      <Card.Root class="opacity-60">
        <Card.Header><Container class="mb-3 size-6" aria-hidden="true" /><Card.Title>{provider}</Card.Title><Card.Description>This provider is not available yet.</Card.Description></Card.Header>
        <Card.Footer><Button variant="outline" disabled>Not available yet</Button></Card.Footer>
      </Card.Root>
    {/each}
  </div>
</div>
