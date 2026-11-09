<script lang="ts">
  import type { HTMLAttributes } from "svelte/elements";
  import { cn } from "$lib/utils.js";

  let { class: className, ...props }: HTMLAttributes<HTMLDivElement> = $props();
</script>

<div data-slot="skeleton" class={cn("rounded-md bg-muted motion-safe:animate-pulse", className)} {...props}></div>
