<script lang="ts">
  import { Github, ArrowRight, GitBranch } from "@lucide/svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
</script>

<svelte:head><title>Source control · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader title="Source control" description="Choose a source provider for your repositories." />
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    <Card.Root>
      <Card.Header><Github class="mb-3 size-6" aria-hidden="true" /><Card.Title>GitHub</Card.Title><Card.Description>Connect your GitHub App and choose which repositories Ayni can access.</Card.Description></Card.Header>
      <Card.Footer><Button href="/setup/github">Configure GitHub <ArrowRight aria-hidden="true" /></Button></Card.Footer>
    </Card.Root>
    {#each ["GitLab", "Bitbucket"] as provider (provider)}
      <Card.Root class="opacity-60">
        <Card.Header><GitBranch class="mb-3 size-6" aria-hidden="true" /><Card.Title>{provider}</Card.Title><Card.Description>This provider is not available yet.</Card.Description></Card.Header>
        <Card.Footer><Button variant="outline" disabled>Not available yet</Button></Card.Footer>
      </Card.Root>
    {/each}
  </div>
</div>
