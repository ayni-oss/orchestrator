<script lang="ts">
  import { Check, ShieldCheck, TriangleAlert, X } from "@lucide/svelte";
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";

  let { data }: PageProps = $props();
  const configuration = $derived(data.certificateConfiguration);
</script>

<svelte:head><title>Ayni · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader title="Ayni" description="Configure signing certificates for Ayni runtime images." />
  <div class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
    <Card.Root>
      <Card.Header>
        <Card.Title class="flex items-center gap-3">
          <ShieldCheck class="size-6 shrink-0" aria-hidden="true" />
          Certificates
        </Card.Title>
      </Card.Header>
      <Card.Content class="flex-1">
        <p class="text-sm text-muted-foreground">Manage certificates used to sign and verify Ayni runtime images.</p>
      </Card.Content>
      <Card.Footer class="mt-auto justify-between gap-3">
        <Button href="/_old/certificates">Configure certificates</Button>
        <span title={configuration.description}>
          {#if configuration.status === "ready"}
            <Check class="size-5 shrink-0 text-emerald-700 dark:text-emerald-300" role="img" aria-label={configuration.description} />
          {:else if configuration.status === "missing"}
            <X class="size-5 shrink-0 text-destructive" role="img" aria-label={configuration.description} />
          {:else}
            <TriangleAlert class="size-5 shrink-0 text-amber-700 dark:text-amber-300" role="img" aria-label={configuration.description} />
          {/if}
        </span>
      </Card.Footer>
    </Card.Root>
  </div>
</div>
