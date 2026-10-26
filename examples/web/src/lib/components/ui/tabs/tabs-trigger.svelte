<script lang="ts">
  import { Tabs as TabsPrimitive } from "bits-ui";
  import { cn } from "$lib/utils.js";
  let { ref = $bindable(null), class: className, ...restProps }: TabsPrimitive.TriggerProps = $props();
</script>
<TabsPrimitive.Trigger bind:ref data-slot="tabs-trigger" class={cn("data-[state=active]:bg-background data-[state=active]:text-foreground focus-visible:border-ring focus-visible:ring-ring/50 inline-flex h-7 items-center justify-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-sm font-medium whitespace-nowrap transition-all focus-visible:ring-[3px] focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50 data-[state=active]:shadow-sm", className)} {...restProps} />
