<script lang="ts">
 import type { Snippet } from 'svelte';
 let { children, badge, actions, actionsLabel }: { children: Snippet; badge?: Snippet; actions?: Snippet; actionsLabel?: string } = $props();
</script>

<div class="flex min-w-0 items-center justify-between gap-3" data-title-row>
 <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2" data-title-content>
  {@render children()}
  {#if badge}{@render badge()}{/if}
 </div>
 {#if actions}<div class="ml-auto flex shrink-0 items-center gap-2" data-title-actions role={actionsLabel ? "group" : undefined} aria-label={actionsLabel}>{@render actions()}</div>{/if}
</div>
