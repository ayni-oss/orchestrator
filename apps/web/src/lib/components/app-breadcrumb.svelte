<script lang="ts">
  import { ChevronLeft } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button/index.js";

  let { context, title, back }: {
    context: string;
    title: string;
    back?: { href: string; label: string };
  } = $props();
</script>

<nav class="breadcrumb" aria-label="Breadcrumb">
  <span class="context" title={context}>{context}</span>
  <span class="separator" aria-hidden="true">/</span>
  <strong aria-current="page">{title}</strong>
  {#if back}
    <Button variant="ghost" size="icon" href={back.href} aria-label={back.label} title={back.label}>
      <ChevronLeft class="size-5" aria-hidden="true" />
    </Button>
  {/if}
</nav>

<style>
  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 18px;
    min-width: 0;
    margin-right: auto;
    font-size: 12px;
    color: var(--muted-foreground);
  }
  .context { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .separator { opacity: .5; }
  strong { font-weight: 400; color: var(--foreground); }
  @media (width <= 540px) {
    .breadcrumb { font-size: 11px; gap: 8px; }
  }
</style>
