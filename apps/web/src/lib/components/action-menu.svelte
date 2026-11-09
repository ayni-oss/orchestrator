<script module lang="ts">
  export type ActionMenuItem = {
    label: string;
    onSelect: () => void;
    disabled?: boolean;
    destructive?: boolean;
  };
</script>

<script lang="ts">
  import { EllipsisVertical } from "@lucide/svelte";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import { Button } from "$lib/components/ui/button/index.js";

  let { label, actions, triggerClass, size = "icon-sm" }: {
    label: string;
    actions: ActionMenuItem[];
    triggerClass?: string;
    size?: "icon-sm" | "icon";
  } = $props();
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button {...props} class={triggerClass} variant="ghost" {size} aria-label={label} title={label}><EllipsisVertical aria-hidden="true" /></Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content align="end" preventScroll={false}>
    {#each actions as action (action.label)}
      <DropdownMenu.Item onSelect={action.onSelect} disabled={action.disabled ?? false} variant={action.destructive ? "destructive" : "default"}>{action.label}</DropdownMenu.Item>
    {/each}
  </DropdownMenu.Content>
</DropdownMenu.Root>
