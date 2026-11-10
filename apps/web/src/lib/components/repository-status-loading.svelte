<script lang="ts">
 import ActivitySkeleton from './activity-skeleton.svelte';
 import { Button } from './ui/button/index.js';
 import { Input } from './ui/input/index.js';
</script>
<div class="space-y-6" aria-busy="true" aria-label="Loading repository status">
 <ActivitySkeleton />
 <section class="space-y-2">
  <h3 class="text-sm font-medium">Inspect a specific commit</h3>
  <div class="space-y-4 rounded-xl border p-4">
   <p class="text-sm text-muted-foreground">Enter an exact commit SHA to check a specific revision. The runtime branch check uses the latest commit on the selected branch.</p>
   <div class="space-y-2"><p class="flex items-center text-sm leading-none font-medium">Commit SHA</p><Input aria-label="Commit SHA" placeholder="Full 40-character lowercase commit SHA" disabled /></div>
   <div class="flex flex-wrap gap-2"><Button disabled>Inspect commit</Button><Button variant="outline" disabled>Refresh inspections</Button></div>
  </div>
 </section>
 {#each ['Previous preparations', 'Inspection history'] as title (title)}
  <details class="rounded-xl border p-4"><summary class="cursor-pointer text-sm font-medium">{title}</summary><div class="space-y-2 pt-4"><ActivitySkeleton count={5} /></div></details>
 {/each}
 <span class="sr-only" role="status">Loading repository status…</span>
</div>
