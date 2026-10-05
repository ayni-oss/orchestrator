<script lang="ts">
  import { Boxes, FolderGit2, Settings } from "@lucide/svelte";
  import { page } from "$app/state";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  const sidebar = Sidebar.useSidebar();
</script>
<Sidebar.Root collapsible="icon" class="shrink-0">
  <Sidebar.Header><a onclick={() => sidebar.setOpenMobile(false)} href="/projects" class="flex h-10 items-center gap-2 px-2 font-semibold no-underline"><Boxes class="size-5 shrink-0" aria-hidden="true" /><span class="group-data-[collapsible=icon]:hidden">Ayni Orchestrator</span></a></Sidebar.Header>
  <Sidebar.Content><Sidebar.Group><Sidebar.GroupContent><Sidebar.Menu><Sidebar.MenuItem><Sidebar.MenuButton isActive={!page.url.pathname.startsWith("/configuration") && !page.url.pathname.startsWith("/setup")} tooltipContent="Projects">{#snippet child({props})}<a onclick={() => sidebar.setOpenMobile(false)} href="/projects" aria-current={!page.url.pathname.startsWith("/configuration") && !page.url.pathname.startsWith("/setup") ? "page" : undefined} {...props}><FolderGit2 aria-hidden="true" /><span>Projects</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem></Sidebar.Menu></Sidebar.GroupContent></Sidebar.Group></Sidebar.Content>
  <Sidebar.Footer><Sidebar.Menu><Sidebar.MenuItem><Sidebar.MenuButton isActive={page.url.pathname.startsWith("/configuration") || page.url.pathname.startsWith("/setup")} tooltipContent="Configuration">{#snippet child({props})}<a onclick={() => sidebar.setOpenMobile(false)} href="/configuration" aria-current={page.url.pathname.startsWith("/configuration") || page.url.pathname.startsWith("/setup") ? "page" : undefined} {...props}><Settings aria-hidden="true" /><span>Configuration</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem></Sidebar.Menu></Sidebar.Footer>
  <Sidebar.Rail />
</Sidebar.Root>
