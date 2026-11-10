<script lang="ts">
 import ActivitySkeleton from './activity-skeleton.svelte';
</script>
<div class="space-y-6" aria-busy="true" aria-label="Loading repository status">
 <ActivitySkeleton />
 {#each ['Previous preparations', 'Inspection history'] as title (title)}
  <details class="rounded-xl border p-4"><summary class="cursor-pointer text-sm font-medium">{title}</summary><div class="space-y-2 pt-4"><ActivitySkeleton count={5} /></div></details>
 {/each}
 <span class="sr-only" role="status">Loading repository status…</span>
</div>
