<script lang="ts">
	import { lockBodyScroll } from "$lib/body-scroll-lock.js";
	import { Dialog as DialogPrimitive } from "bits-ui";

	let { open = $bindable(false), ...restProps }: DialogPrimitive.RootProps = $props();
	$effect(() => {if (open) return lockBodyScroll(document);});
</script>

<DialogPrimitive.Root bind:open {...restProps} />
