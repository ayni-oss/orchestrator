<script lang="ts">
  import { Boxes, FolderGit2, BookOpen, GitBranch, Container } from "@lucide/svelte";
  import { page } from "$app/state";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  const sidebar = Sidebar.useSidebar();
  const repositoriesActive = $derived(page.url.pathname.startsWith("/repositories") || page.url.pathname.startsWith("/profiles"));
  const sourceControlActive = $derived(page.url.pathname.startsWith("/source-control") || page.url.pathname.startsWith("/setup/github"));
  const registriesActive = $derived(page.url.pathname.startsWith("/container-registries"));
  const projectsActive = $derived(page.url.pathname.startsWith("/projects") || page.url.pathname.startsWith("/tasks"));
</script>
<Sidebar.Root collapsible="icon" class="shrink-0">
  <Sidebar.Header><a onclick={() => sidebar.setOpenMobile(false)} href="/projects" class="flex h-10 items-center gap-2 px-2 font-semibold no-underline"><Boxes class="size-5 shrink-0" aria-hidden="true" /><span class="group-data-[collapsible=icon]:hidden">Ayni Orchestrator</span></a></Sidebar.Header>
  <Sidebar.Content><Sidebar.Group><Sidebar.GroupContent><Sidebar.Menu>
    <Sidebar.MenuItem><Sidebar.MenuButton isActive={projectsActive} tooltipContent="Projects">{#snippet child({props})}<a {...props} onclick={() => sidebar.setOpenMobile(false)} href="/projects" aria-current={projectsActive ? "page" : undefined}><FolderGit2 aria-hidden="true" /><span>Projects</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem>
    <Sidebar.MenuItem><Sidebar.MenuButton isActive={repositoriesActive} tooltipContent="Repositories">{#snippet child({props})}<a {...props} onclick={() => sidebar.setOpenMobile(false)} href="/repositories" aria-current={repositoriesActive ? "page" : undefined}><BookOpen aria-hidden="true" /><span>Repositories</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem>

    <Sidebar.MenuItem><Sidebar.MenuButton isActive={sourceControlActive} tooltipContent="Sources">{#snippet child({props})}<a {...props} onclick={() => sidebar.setOpenMobile(false)} href="/source-control" aria-current={sourceControlActive ? "page" : undefined}><GitBranch aria-hidden="true" /><span>Sources</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem>
    <Sidebar.MenuItem><Sidebar.MenuButton isActive={registriesActive} tooltipContent="Destinations">{#snippet child({props})}<a {...props} onclick={() => sidebar.setOpenMobile(false)} href="/container-registries" aria-current={registriesActive ? "page" : undefined}><Container aria-hidden="true" /><span>Destinations</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem>
  </Sidebar.Menu></Sidebar.GroupContent></Sidebar.Group></Sidebar.Content>
  <Sidebar.Rail />
</Sidebar.Root>
