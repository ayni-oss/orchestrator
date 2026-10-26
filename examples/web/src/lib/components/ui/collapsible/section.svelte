<script lang="ts">
  import { Collapsible } from "bits-ui";
  import { ChevronDown } from "@lucide/svelte";
  import type { Snippet } from "svelte";
  let { title, children }: { title: string; children: Snippet } = $props();
  let open = $state(false);
</script>

<Collapsible.Root bind:open class="border-border rounded-lg border p-3">
  <Collapsible.Trigger class="focus-visible:ring-ring/50 flex w-full items-center justify-between rounded-md text-left text-sm font-medium outline-none focus-visible:ring-3">
    {title}<ChevronDown class={['size-4 text-muted-foreground transition-transform', open && 'rotate-180']} aria-hidden="true" />
  </Collapsible.Trigger>
  <Collapsible.Content>{@render children()}</Collapsible.Content>
</Collapsible.Root>
