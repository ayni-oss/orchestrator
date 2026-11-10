<script lang="ts">
 let { failed = false }: { failed?: boolean } = $props();
 import Skeleton from './ui/skeleton/skeleton.svelte';
 import { Button } from './ui/button/index.js';
</script>
<section class="space-y-5" aria-busy={!failed} aria-label="Loading runtime onboarding">
 <ol class="grid gap-2 sm:grid-cols-5" aria-label="Runtime onboarding steps">
  {#each ['Connect integrations', 'Check ayni.lock', 'Review and start', 'Prepare image', 'Verify and finish'] as step, index (step)}
   <li class="min-w-0"><Button variant="outline" class="h-full min-h-10 w-full justify-start whitespace-normal text-left" disabled>{index + 1}. {step}</Button></li>
  {/each}
 </ol>
 <div class="min-h-96 space-y-4 rounded-xl border bg-card p-5" aria-hidden={failed ? undefined : true}>
  {#if failed}<p role="alert">Runtime onboarding could not be loaded. Retry to check the saved status.</p>{:else}
  <div class="flex justify-between"><Skeleton class="h-7 w-48" /><Skeleton class="size-8" /></div>
  <div class="space-y-6 rounded-md border p-4"><Skeleton class="h-5 w-2/3" /><Skeleton class="h-5 w-1/2" /><Skeleton class="h-5 w-2/3" /></div>
  <Skeleton class="h-20 w-full" />
  <div class="flex justify-between border-t pt-4"><Skeleton class="h-8 w-16" /><Skeleton class="h-8 w-16" /></div>
  {/if}
 </div>
 {#if !failed}<span class="sr-only" role="status">Loading runtime onboarding…</span>{/if}
</section>
