<script lang="ts">
  import type { Snippet } from "svelte";
  let { eyebrow, title, description, actions, flush = false }: { eyebrow?: string; title: string; description?: string; actions?: Snippet; flush?: boolean } = $props();
</script>

<header class="page-header" class:flush>
  <div class="min-w-0">
    {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
    <h1>{title}</h1>
    {#if description}<p class="description">{description}</p>{/if}
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  .page-header{display:flex;align-items:flex-start;justify-content:space-between;gap:24px;margin-bottom:34px}
  .page-header.flush{margin-bottom:0}
  .eyebrow{font:10px var(--font-mono);letter-spacing:1.5px;text-transform:uppercase;color:var(--muted-foreground);margin-bottom:13px}
  h1{font-size:36px;font-weight:600;letter-spacing:-1px;line-height:1.15;overflow-wrap:anywhere}
  .description{max-width:720px;font-size:13px;line-height:1.7;color:var(--muted-foreground);margin-top:10px}
  .actions{display:flex;flex-shrink:0;flex-wrap:wrap;align-items:center;gap:8px}
  @media(max-width:700px){.page-header{gap:16px;flex-wrap:wrap;margin-bottom:24px}h1{font-size:30px}}
</style>
