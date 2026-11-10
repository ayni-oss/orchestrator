<script lang="ts" generics="Item">
 import { onMount, type Snippet } from 'svelte';
 import { Button } from '$lib/components/ui/button/index.js';
 import * as Pagination from '$lib/components/ui/pagination/index.js';
 import { historyBatchSize, type ReadHistoryPage } from '$lib/repository-history';

 let { label, readPage, empty, children }: {
  label: string;
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
 const lastCachedPage = $derived(Math.ceil((items?.length ?? 0) / itemsPerPage));
 const visibleItems = $derived(items?.slice((page - 1) * itemsPerPage, page * itemsPerPage));

 async function fetchBatch() {
  if (loading || (items && !next)) return;
  loading = true;
  failed = false;
  try {
   const response = await readPage(next);
   if (!mounted) return;
   items = [...(items ?? []), ...response.items];
   next = response.next;
  } catch {
   if (mounted) failed = true;
  } finally {
   if (mounted) loading = false;
  }
 }

 function prefetchAtEnd(target: number) {
  if (target === lastCachedPage && next) void fetchBatch();
 }

 onMount(() => {
  mounted = true;
  void fetchBatch();
  return () => { mounted = false; };
 });
</script>

<div class="space-y-2" aria-busy={loading}>
 {#if visibleItems?.length}
  {@render children(visibleItems)}
 {:else if items}
  <p class="text-sm text-muted-foreground">{empty}</p>
 {/if}
 {#if loading}
  <p role="status" class="text-sm text-muted-foreground">{items ? 'Loading more history…' : 'Loading history…'}</p>
 {:else if failed}
  <div class="flex flex-wrap items-center gap-2">
   <p role="alert" class="text-sm">Could not load {items ? 'more history' : 'history'}.</p>
   <Button variant="outline" size="sm" onclick={fetchBatch}>Retry</Button>
  </div>
 {/if}
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
