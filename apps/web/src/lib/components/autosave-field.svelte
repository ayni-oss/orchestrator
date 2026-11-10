<script lang="ts">
  import type { Snippet } from 'svelte';
  import { Check, Save } from '@lucide/svelte';
  import InputGroup from '$lib/components/ui/input-group/input-group.svelte';

  let { children, saving = false, savedRevision, class: className }: {
    children: Snippet;
    saving?: boolean;
    savedRevision?: number;
    class?: string;
  } = $props();

  const state = $derived(saving ? 'saving' : savedRevision ? 'saved' : 'idle');
</script>

<InputGroup class={className}>
  {@render children()}
  <span data-slot="input-group-addon" data-align="inline-end" data-autosave-state={state}
    class="flex w-8 shrink-0 items-center justify-center text-muted-foreground" role="status">
    {#if saving}
      <Save class="size-4 motion-safe:animate-pulse" aria-hidden="true" />
      <span class="sr-only">Saving</span>
    {:else if savedRevision}
      {#key savedRevision}
        <span class="confirmed text-primary">
          <Check class="size-4" aria-hidden="true" />
        </span>
      {/key}
      <span class="sr-only">Saved</span>
    {/if}
  </span>
</InputGroup>

<style>
  @media (prefers-reduced-motion: no-preference) {
    .confirmed {
      animation: confirm 180ms ease-out both;
    }
  }

  @keyframes confirm {
    from { opacity: 0; transform: scale(0.8); }
    to { opacity: 1; transform: scale(1); }
  }
</style>
