<script lang="ts">
  import { page } from "$app/state";
  import { Button } from "$lib/components/ui/button/index.js";
</script>

<svelte:head><title>Unable to load · Ayni Orchestrator</title></svelte:head>
<section class="mx-auto max-w-xl rounded-lg border bg-card p-8 text-center" role="alert">
  <p class="text-sm font-medium text-destructive">Error {page.status}</p>
  <h1 class="mt-2 text-2xl font-semibold">This view could not be loaded</h1>
  <p class="mt-3 text-sm text-muted-foreground">{page.error?.message ?? "An unexpected browser server error occurred."}</p>
  <div class="mt-6 flex justify-center gap-2"><Button href="/">Return to overview</Button><Button variant="outline" onclick={() => location.reload()}>Try again</Button></div>
</section>
