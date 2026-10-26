<script lang="ts">
  import { Check, ChevronRight, TriangleAlert, X } from "@lucide/svelte";

  let { status, dropdown = false, expanded = false }: {
    status?: { status: string; description: string } | undefined;
    dropdown?: boolean;
    expanded?: boolean;
  } = $props();
</script>

<span class="ml-auto flex shrink-0 items-center gap-2 group-data-[collapsible=icon]:hidden">
  {#if status}
    <span class="flex size-4 shrink-0 items-center" title={status.description}>
      {#if status.status === "ready"}
        <Check class="size-4 text-emerald-700 dark:text-emerald-300" role="img" aria-label={status.description} />
      {:else if status.status === "missing"}
        <X class="size-4 text-destructive" role="img" aria-label={status.description} />
      {:else}
        <TriangleAlert class="size-4 text-amber-700 dark:text-amber-300" role="img" aria-label={status.description} />
      {/if}
    </span>
  {/if}
  {#if dropdown}
    <span class="flex size-4 shrink-0 items-center" aria-hidden="true">
      <ChevronRight class={['size-4 transition-transform', expanded && 'rotate-90']} />
    </span>
  {/if}
</span>
