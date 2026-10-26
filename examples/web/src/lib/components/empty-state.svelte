<script lang="ts">
  import type { Snippet } from "svelte";
  let { title, description, action }: { title: string; description: string; action?: Snippet } = $props();
</script>

<div class="flex min-h-48 flex-col items-center justify-center rounded-lg border border-dashed bg-muted/20 p-8 text-center">
  <h2 class="text-base font-semibold">{title}</h2>
  <p class="mt-2 max-w-md text-sm text-muted-foreground">{description}</p>
  {#if action}<div class="mt-4">{@render action()}</div>{/if}
</div>
