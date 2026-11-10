<script lang="ts">
 import { Info } from '@lucide/svelte';
 import { Tooltip } from 'bits-ui';
 import { Button, type ButtonSize } from './ui/button/index.js';
 let { label, rows, size = 'icon-sm', disabled = false }: {
  label: string;
  rows: { label: string; value?: string; monospace?: boolean }[];
  size?: ButtonSize;
  disabled?: boolean;
 } = $props();
</script>

<Tooltip.Provider>
 <Tooltip.Root delayDuration={250}>
  <Tooltip.Trigger>
   {#snippet child({props})}
    <Button {...props} variant="ghost" {size} {disabled} aria-label={label}><Info aria-hidden="true" /></Button>
   {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Portal>
   <Tooltip.Content role="tooltip" align="end" sideOffset={4} class="z-50 max-w-[min(24rem,calc(100vw-2rem))] rounded-md border bg-popover p-2 text-xs text-popover-foreground shadow-md">
    <dl class="space-y-1">
     {#each rows as row (row.label)}
      {#if row.value}
       <div>
        <dt class="text-muted-foreground">{row.label}</dt>
        <dd class={['break-all', row.monospace !== false && 'font-mono']}>{row.value}</dd>
       </div>
      {/if}
     {/each}
    </dl>
   </Tooltip.Content>
  </Tooltip.Portal>
 </Tooltip.Root>
</Tooltip.Provider>
