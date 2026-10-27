<script module lang="ts">
  import type { Snippet } from "svelte";
  export type InformationRow = { label: string; value: string | Snippet; monospace?: boolean };
</script>

<script lang="ts">
  let { rows }: { rows: InformationRow[] } = $props();
</script>

<dl class="min-w-0 space-y-2 text-sm">
  {#each rows as row (row.label)}
    <div class="flex min-h-8 flex-wrap items-center gap-x-6 gap-y-1">
      <dt class="min-w-32 max-w-full shrink-0 font-normal text-muted-foreground">{row.label}</dt>
      <dd class={['min-w-0 max-w-full flex-[1_0_max-content] [overflow-wrap:anywhere]', row.monospace && 'font-mono text-xs']}>
        {#if typeof row.value === "string"}{row.value}{:else}{@render row.value()}{/if}
      </dd>
    </div>
  {/each}
</dl>
