<script lang="ts">
 import Skeleton from '$lib/components/ui/skeleton/skeleton.svelte';
 let { labels, multiline = false }: { labels: string[]; multiline?: boolean } = $props();
</script>
<div class="space-y-5" aria-busy="true" aria-label="Loading fields">
 {#each labels as label (label)}
  <div class="space-y-2"><p class="text-sm font-medium">{label}</p><Skeleton class={multiline ? 'h-24 w-full' : 'h-8 w-full'} /></div>
 {/each}
</div>
