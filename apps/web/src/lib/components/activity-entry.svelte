<script lang="ts">
 import { Ellipsis, Info } from '@lucide/svelte';
 import { Tooltip } from 'bits-ui';
 import type { Snippet } from 'svelte';
 import StatusBadge from '$lib/components/status-badge.svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import * as Dialog from '$lib/components/ui/dialog/index.js';
 import type { StatusTone } from '$lib/presentation.js';

 let { label, updatedAtUnixMs, identifiers, title, description, statuses, statusesFirst = false, information, details, actions }: {
  label: string;
  updatedAtUnixMs?: string;
  identifiers: {label: string; value?: string}[];
  title: string;
  description: string;
  statuses: {label: string; status: string; tone: StatusTone}[];
  statusesFirst?: boolean;
  information: Snippet;
  details: Snippet<[string]>;
  actions?: Snippet;
 } = $props();

 const updated = $derived(updatedAtUnixMs ? new Date(Number(updatedAtUnixMs)) : undefined);
 const fullTime = $derived(updated?.toLocaleString() || 'Not retained');
</script>

{#snippet statusLine()}
 <div class="flex flex-wrap gap-x-3 gap-y-1">
  {#each statuses as entry (entry.label)}
   <span class="inline-flex items-center gap-1">
    <span class="text-muted-foreground">{entry.label}</span>
    <StatusBadge status={entry.status} tone={entry.tone} />
   </span>
  {/each}
 </div>
{/snippet}

<article class="flex items-center gap-2 rounded-md border px-3 py-2" aria-label={label}>
 <div class="min-w-0 flex-1 space-y-1 text-xs">
  {#if statusesFirst}{@render statusLine()}{/if}
  <div>{@render information()}</div>
  {#if !statusesFirst}{@render statusLine()}{/if}
 </div>
 <div class="flex shrink-0 flex-col items-end gap-1">
  <div class="flex items-center gap-1">
   <Tooltip.Root delayDuration={250}>
    <Tooltip.Trigger>
     {#snippet child({props})}
      <Button {...props} variant="ghost" size="icon-sm" aria-label="View identifiers"><Info aria-hidden="true" /></Button>
     {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Portal>
     <Tooltip.Content role="tooltip" align="end" sideOffset={4} class="z-50 max-w-[min(24rem,calc(100vw-2rem))] rounded-md border bg-popover p-2 text-xs text-popover-foreground shadow-md">
      <dl class="space-y-1">
       {#each identifiers as identifier (identifier.label)}
        {#if identifier.value}
         <div>
          <dt class="text-muted-foreground">{identifier.label}</dt>
          <dd class="break-all font-mono">{identifier.value}</dd>
         </div>
        {/if}
       {/each}
      </dl>
     </Tooltip.Content>
    </Tooltip.Portal>
   </Tooltip.Root>
   <Dialog.Root>
    <Dialog.Trigger>
     {#snippet child({props})}
      <Button {...props} variant="ghost" size="icon-sm" aria-label={`View ${title.toLowerCase()}`} title={`View ${title.toLowerCase()}`}><Ellipsis aria-hidden="true" /></Button>
     {/snippet}
    </Dialog.Trigger>
    <Dialog.Content class="max-h-[85dvh] grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden sm:max-w-2xl" preventScroll>
     <Dialog.Header class="pr-8">
      <Dialog.Title>{title}</Dialog.Title>
      <Dialog.Description>{description}</Dialog.Description>
     </Dialog.Header>
     <div class="space-y-3 overflow-y-auto pr-1">
      {@render details(fullTime)}
     </div>
     {#if actions}<Dialog.Footer>{@render actions()}</Dialog.Footer>{/if}
    </Dialog.Content>
   </Dialog.Root>
  </div>
  {#if updated}
   <time class="text-xs text-muted-foreground" datetime={updated.toISOString()} title={`Updated ${fullTime}`}>
    {updated.toLocaleString(undefined, {dateStyle:'short', timeStyle:'short'})}
   </time>
  {/if}
 </div>
</article>
