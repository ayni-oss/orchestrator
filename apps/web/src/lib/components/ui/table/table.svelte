<script lang="ts">
	import { cn, type WithElementRef } from "$lib/utils.js";
	import type { HTMLTableAttributes } from "svelte/elements";

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLTableAttributes> = $props();
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex (keyboard access to horizontally scrollable tabular content) -->
<div data-slot="table-container" class="relative w-full overflow-x-auto" role="region" aria-label="Scrollable table" tabindex="0">
	<table bind:this={ref} data-slot="table" class={cn("w-full caption-bottom text-sm", className)} {...restProps}>
		{@render children?.()}
	</table>
</div>
