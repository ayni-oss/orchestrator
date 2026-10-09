<script lang="ts">
  import { ChevronRight } from "@lucide/svelte";
  import { mergeProps } from "bits-ui";
  import type { Snippet } from "svelte";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as Collapsible from "$lib/components/ui/collapsible/index.js";

  let { title, icon, children }: { title: string; icon: Snippet; children: Snippet } = $props();
  let open = $state(true);
  const sidebar = Sidebar.useSidebar();

  function expandSidebar() {
    if (!sidebar.isMobile && sidebar.state === "collapsed") {
      sidebar.setOpen(true);
      open = true;
    }
  }
</script>

<Sidebar.MenuItem>
  <Collapsible.Root bind:open>
    <Collapsible.Trigger>
      {#snippet child({ props })}
        <Sidebar.MenuButton
          {...mergeProps(props, { onclick: expandSidebar })}
          tooltipContent={title}
        >
          {@render icon()}
          <span>{title}</span>
          <ChevronRight class={['ml-auto transition-transform group-data-[collapsible=icon]:hidden', open && 'rotate-90']} aria-hidden="true" />
        </Sidebar.MenuButton>
      {/snippet}
    </Collapsible.Trigger>
    <Collapsible.Content>
      <Sidebar.MenuSub class="mx-2 px-1">{@render children()}</Sidebar.MenuSub>
    </Collapsible.Content>
  </Collapsible.Root>
</Sidebar.MenuItem>
