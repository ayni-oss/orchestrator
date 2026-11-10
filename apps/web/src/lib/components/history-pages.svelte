<script lang="ts" generics="Item">
 import ActivitySkeleton from './activity-skeleton.svelte';
 import { onMount, untrack, type Snippet } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import * as Pagination from '$lib/components/ui/pagination/index.js';
 import { historyBatchSize, type ReadHistoryPage } from '$lib/repository-history';

 let { label, readPage, empty, revision = 0, children }: {
  label: string;
  revision?: number;
  readPage: ReadHistoryPage<Item>;
  empty: string;
  children: Snippet<[Item[]]>;
 } = $props();

 const itemsPerPage = historyBatchSize / 2;
 let items = $state.raw<Item[]>();
 let next = $state('');
 let page = $state(1);
 let loading = $state(false);
 let failed = $state(false);
 let mounted = false;
 let requestSequence = 0;
 let failedReset = false;
 let seenRevision = untrack(() => revision);
 const lastCachedPage = $derived(Math.ceil((items?.length ?? 0) / itemsPerPage));
 const visibleItems = $derived(items?.slice((page - 1) * itemsPerPage, page * itemsPerPage));

 async function fetchBatch(reset = false) {
  if (!reset && (loading || (items && !next))) return;
  const sequence = ++requestSequence;
  loading = true;
  failed = false;
  try {
   const response = await readPage(reset ? '' : next);
   if (!mounted || sequence !== requestSequence) return;
   items = [...(reset ? [] : items ?? []), ...response.items];
   if (reset) page = 1;
   next = response.next;
  } catch {
   if (mounted && sequence === requestSequence) { failed = true; failedReset = reset; }
  } finally {
   if (mounted && sequence === requestSequence) loading = false;
  }
 }

 function prefetchAtEnd(target: number) {
  if (target === lastCachedPage && next) void fetchBatch();
 }

 $effect(() => {
  const currentRevision = revision;
  untrack(() => {
   if (currentRevision === seenRevision) return;
   seenRevision = currentRevision;
   if (mounted) void fetchBatch(true);
  });
 });

 onMount(() => {
  mounted = true;
  void fetchBatch();
  return () => { mounted = false; };
 });
</script>

<div class="space-y-2" aria-busy={loading}>
 <div class="min-h-[362px] space-y-2">
 {#if visibleItems?.length}
  {@render children(visibleItems)}
 {:else if !items && !failed}
  <ActivitySkeleton count={5} />
 {:else if items}
  <p class="text-sm text-muted-foreground">{empty}</p>
 {/if}
 </div>
 <div class="min-h-8">
 {#if loading}
  <p role="status" class="sr-only">{items ? 'Loading more history…' : 'Loading history…'}</p>
 {:else if failed}
  <div class="flex flex-wrap items-center gap-2">
   <p role="alert" class="text-sm">Could not load {items ? 'more history' : 'history'}.</p>
   <Button variant="outline" size="sm" onclick={() => fetchBatch(failedReset)}>Retry</Button>
  </div>
 {/if}
 </div>
 <div class="min-h-8">
 {#if items && lastCachedPage > 1}
  <Pagination.Root count={items.length} perPage={itemsPerPage} bind:page onPageChange={prefetchAtEnd} aria-label={label}>
   <Pagination.Content>
    <Pagination.Item><Pagination.Previous /></Pagination.Item>
    <Pagination.Item><span class="px-2 text-sm tabular-nums" aria-live="polite">Page {page}</span></Pagination.Item>
    <Pagination.Item><Pagination.Next /></Pagination.Item>
   </Pagination.Content>
  </Pagination.Root>
 {/if}
 </div>
</div>
