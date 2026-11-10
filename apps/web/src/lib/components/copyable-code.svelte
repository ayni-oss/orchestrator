<script lang="ts">
 import { Copy } from '@lucide/svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 let { value, label, oncopy, disabled = false }: {
  value: string;
  label: string;
  oncopy?: () => void | Promise<void>;
  disabled?: boolean;
 } = $props();
</script>

<div class="flex items-start gap-2 rounded-md border bg-muted/30 p-2">
 <pre class="min-w-0 flex-1 px-1 py-2 text-xs whitespace-pre-wrap break-all"><code>{value}</code></pre>
 {#if oncopy}
  <Button variant="outline" size="icon" aria-label={label} title={label} {disabled} onclick={oncopy}>
   <Copy aria-hidden="true" />
  </Button>
 {/if}
</div>
