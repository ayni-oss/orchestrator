<script lang="ts">
  import { Check, TriangleAlert, X } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  let { data }: PageProps = $props();
  const providers = [{"name": "GitHub", "logo": "github", "description": "Store repository testing environments in GitHub Container Registry.", "href": "/_old/container-registries/github"}, {"name": "Docker Hub", "logo": "docker", "description": "Store testing environment images in Docker Hub.", "href": null}, {"name": "Amazon ECR", "logo": "aws", "description": "Store testing environment images in Amazon Elastic Container Registry.", "href": null}];
</script>

<svelte:head><title>Container registries · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader title="Container registries" description="Choose a registry destination for your testing environment images." />

  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    {#each providers as provider (provider.name)}
      <Card.Root>
        <Card.Header>
          <Card.Title class="flex items-center gap-3">
            <img src={`/provider-logos/${provider.logo}.svg`} alt="" class="size-6 shrink-0 invert dark:invert-0" />
            {provider.name}
          </Card.Title>
        </Card.Header>
        <Card.Content class="flex-1"><p class="text-sm text-muted-foreground">{provider.description}</p></Card.Content>
        <Card.Footer class="mt-auto justify-between gap-3">
          {#if provider.href}
            <Button href={provider.href}>Configure {provider.name}</Button>
            {#if provider.logo === "github"}
              <span title={data.registryConfiguration.description}>
                {#if data.registryConfiguration.status === "ready"}
                  <Check class="size-5 shrink-0 text-emerald-700 dark:text-emerald-300" role="img" aria-label={data.registryConfiguration.description} />
                {:else if data.registryConfiguration.status === "missing"}
                  <X class="size-5 shrink-0 text-destructive" role="img" aria-label={data.registryConfiguration.description} />
                {:else}
                  <TriangleAlert class="size-5 shrink-0 text-amber-700 dark:text-amber-300" role="img" aria-label={data.registryConfiguration.description} />
                {/if}
              </span>
            {/if}
          {:else}
            <Button variant="outline" disabled class="disabled:opacity-70">Not available</Button>
          {/if}
        </Card.Footer>
      </Card.Root>
    {/each}
  </div>
</div>
