<script lang="ts">
  import TitleRow from "./title-row.svelte";
  import Skeleton from "./ui/skeleton/skeleton.svelte";
  import type { Snippet } from "svelte";

  let { eyebrow, title, titleMark, titleBadge, actions, loading = false }: { eyebrow?: string; title: string; titleMark?: Snippet; titleBadge?: Snippet; actions?: Snippet; loading?: boolean } = $props();
</script>

<header class="page-header">
  {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
  <TitleRow badge={titleBadge} {actions}>
    <h1 class:marked={titleMark} aria-busy={loading}>
      {#if titleMark}{@render titleMark()}{/if}
      {#if loading}<span class="sr-only">{title}</span><Skeleton class="h-[1.15em] w-[min(20rem,55vw)]" />{:else}<span>{title}</span>{/if}
    </h1>
  </TitleRow>
</header>

<style>
  .page-header { margin-bottom: 34px; }
  .eyebrow { font: 10px var(--font-mono); letter-spacing: 1.5px; text-transform: uppercase; color: var(--muted-foreground); margin-bottom: 13px; }
  h1.marked { display: flex; align-items: center; gap: 12px; }
  h1 { min-width: 0; font-size: 36px; font-weight: 600; letter-spacing: -1px; line-height: 1.15; overflow-wrap: anywhere; }
  @media (width <= 700px) {
    .page-header { margin-bottom: 24px; }
    h1 { font-size: 30px; }
  }
</style>
