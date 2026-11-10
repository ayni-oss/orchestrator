<script lang="ts">
  import type { Snippet } from "svelte";

  let { eyebrow, title, titleMark, actions }: { eyebrow?: string; title: string; titleMark?: Snippet; actions?: Snippet } = $props();
</script>

<header class="page-header">
  {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
  <div class="title-row">
    <h1 class:marked={titleMark}>{#if titleMark}{@render titleMark()}{/if}<span>{title}</span></h1>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </div>
</header>

<style>
  .page-header { margin-bottom: 34px; }
  .eyebrow { font: 10px var(--font-mono); letter-spacing: 1.5px; text-transform: uppercase; color: var(--muted-foreground); margin-bottom: 13px; }
  .title-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  h1.marked { display: flex; align-items: center; gap: 12px; }
  h1 { min-width: 0; font-size: 36px; font-weight: 600; letter-spacing: -1px; line-height: 1.15; overflow-wrap: anywhere; }
  .actions { display: flex; flex-shrink: 0; align-items: center; gap: 8px; }
  @media (width <= 700px) {
    .page-header { margin-bottom: 24px; }
    h1 { font-size: 30px; }
  }
</style>
