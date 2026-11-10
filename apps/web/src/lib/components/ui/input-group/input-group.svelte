<script lang="ts">
  import type { Snippet } from 'svelte';
  import { cn } from '$lib/utils.js';

  let { children, class: className }: { children: Snippet; class?: string } = $props();
</script>

<div data-slot="input-group" role="group" class={cn('relative flex w-full min-w-0 items-center rounded-lg border border-input bg-transparent transition-colors dark:bg-input/30', className)}>
  {@render children()}
</div>

<style>
  div:has(:global([data-slot="input-group-control"]:focus-visible)) {
    border-color: var(--ring);
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--ring) 50%, transparent);
  }

  div :global([data-slot="input-group-control"]:where(:not([type="checkbox"]))) {
    background: transparent;
    border: 0;
    border-radius: 0;
    box-shadow: none;
    padding-inline-end: 2rem;
  }

  div:has(:global([data-slot="input-group-control"]:where(:not([type="checkbox"])))) > :global([data-slot="input-group-addon"]) {
    position: absolute;
    inset-inline-end: 0;
    pointer-events: none;
  }

  div:has(> :global(textarea)) > :global([data-slot="input-group-addon"]) {
    inset-block-start: 0.5rem;
  }

  div > :global(textarea[data-slot="input-group-control"]) {
    resize: vertical;
  }

  div :global(select[data-slot="input-group-control"]) {
    padding-inline-end: 3.75rem;
  }

  div:has(:global(select[data-slot="input-group-control"])) > :global([data-slot="input-group-addon"]) {
    inset-inline-end: 1.75rem;
  }
</style>
