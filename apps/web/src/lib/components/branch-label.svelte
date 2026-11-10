<script lang="ts">
 import { GitBranch } from '@lucide/svelte';
 import { buttonVariants } from '$lib/components/ui/button/index.js';
 let { name, label = 'Branch' }: { name: string; label?: string } = $props();
</script>

<span class={buttonVariants({variant:'secondary',class:'pointer-events-none h-auto min-h-8 max-w-full whitespace-normal py-1'})}>
 <GitBranch aria-hidden="true" />
 <span class="sr-only">{label}</span>
 <span class="break-all">{name}</span>
</span>
