<script module lang="ts">
  import type { Snippet } from "svelte";
  export type InformationRow = { label: string; value: string | Snippet; monospace?: boolean };
</script>

<script lang="ts">
  let { rows, aligned = false }: { rows: InformationRow[]; aligned?: boolean } = $props();
</script>

<dl class={['min-w-0 text-sm', aligned ? 'grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-x-6 gap-y-2 sm:grid-cols-[max-content_minmax(0,1fr)]' : 'space-y-2']}>
  {#each rows as row (row.label)}
    <div class={['min-h-8 items-center', aligned ? 'col-span-2 grid grid-cols-subgrid' : 'flex flex-wrap gap-x-6 gap-y-1']}>
      <dt class={['max-w-full font-normal text-muted-foreground', aligned ? 'min-w-0' : 'min-w-32 shrink-0']}>{row.label}</dt>
      <dd class={['min-w-0 max-w-full [overflow-wrap:anywhere]', !aligned && 'flex-[1_0_max-content]', row.monospace && 'font-mono text-xs']}>
        {#if typeof row.value === "string"}{row.value}{:else}{@render row.value()}{/if}
      </dd>
    </div>
  {/each}
</dl>
