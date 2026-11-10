<script lang="ts">
 import type { Snippet } from 'svelte';
 let { children }: { children: Snippet } = $props();
</script>
<div class="mx-auto w-full max-w-5xl space-y-8">{@render children()}</div>
