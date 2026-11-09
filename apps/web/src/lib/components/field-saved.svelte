<script lang="ts">
  import { Check, Save } from '@lucide/svelte';

  let { revision }: { revision?: number } = $props();
</script>

<span role="status" class="pointer-events-none absolute right-1 bottom-0 translate-y-1/2">
  {#if revision}
    {#key revision}
      <span class="saved-icon relative block rounded-sm bg-card p-1 text-foreground shadow-sm" aria-hidden="true">
        <Save class="size-4" />
        <Check class="absolute right-0 bottom-0 size-3 rounded-sm bg-card" />
      </span>
      <span class="sr-only">Saved</span>
    {/key}
  {/if}
</span>

<style>
  .saved-icon {
    animation: saved 1.8s ease-out both;
  }

  @keyframes saved {
    0% { opacity: 0; transform: scale(.8); }
    12%, 78% { opacity: 1; transform: scale(1); }
    100% { opacity: 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .saved-icon { animation-timing-function: step-end; animation-name: saved-still; }
  }

  @keyframes saved-still {
    from { opacity: 1; }
    to { opacity: 0; }
  }
</style>
