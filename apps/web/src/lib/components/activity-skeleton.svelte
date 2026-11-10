<script lang="ts">
 import Skeleton from '$lib/components/ui/skeleton/skeleton.svelte';
 let { count = 1 }: { count?: number } = $props();
</script>
{#each Array.from({length: count}, (_, i) => i) as row (row)}
 <div class="flex min-h-[66px] items-center gap-2 rounded-md border px-3 py-2" aria-hidden="true">
  <div class="min-w-0 flex-1 space-y-1">
   <div class="flex flex-wrap gap-3"><Skeleton class="h-5 w-28" /><Skeleton class="h-5 w-28" /><Skeleton class="h-5 w-24" /></div>
   <Skeleton class="h-4 w-36" />
  </div>
  <div class="flex shrink-0 flex-col items-end gap-1"><div class="flex gap-1"><Skeleton class="size-7" /><Skeleton class="size-7" /></div><Skeleton class="h-4 w-28" /></div>
 </div>
{/each}
