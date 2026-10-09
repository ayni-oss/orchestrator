<script lang="ts">
  import { Boxes, FolderGit2, BookOpen, GitBranch, Github, Container, ShieldCheck, Plug, Layers, Wrench } from "@lucide/svelte";
  import { page } from "$app/state";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import SidebarNavGroup from "$lib/components/sidebar-nav-group.svelte";

  const sidebar = Sidebar.useSidebar();
  const repositoriesActive = $derived(page.url.pathname.startsWith("/repositories") || page.url.pathname.startsWith("/profiles"));
  const sourceControlActive = $derived(page.url.pathname === "/source-control");
  const githubActive = $derived(page.url.pathname.startsWith("/source-control/github") || page.url.pathname.startsWith("/setup/github"));
  const registriesActive = $derived(page.url.pathname.startsWith("/container-registries"));
  const certificatesActive = $derived(page.url.pathname.startsWith("/certificates"));
  const projectsActive = $derived(page.url.pathname.startsWith("/projects") || page.url.pathname.startsWith("/tasks"));
</script>

<Sidebar.Root collapsible="icon" class="shrink-0">
  <Sidebar.Header>
    <a onclick={() => sidebar.setOpenMobile(false)} href="/projects" class="flex h-10 items-center gap-2 px-2 font-semibold no-underline">
      <Boxes class="size-5 shrink-0" aria-hidden="true" />
      <span class="group-data-[collapsible=icon]:hidden">Ayni Orchestrator</span>
    </a>
  </Sidebar.Header>
  <Sidebar.Content>
    <Sidebar.Group>
      <Sidebar.GroupContent>
        <Sidebar.Menu>
          <Sidebar.MenuItem>
            <Sidebar.MenuButton isActive={projectsActive} tooltipContent="Projects">
              {#snippet child({ props })}
                <a {...props} onclick={() => sidebar.setOpenMobile(false)} href="/projects" aria-current={projectsActive ? "page" : undefined}>
                  <FolderGit2 aria-hidden="true" /><span>Projects</span>
                </a>
              {/snippet}
            </Sidebar.MenuButton>
          </Sidebar.MenuItem>
          <SidebarNavGroup title="Integrations">
            {#snippet icon()}<Plug aria-hidden="true" />{/snippet}
            <Sidebar.MenuSubItem>
              <Sidebar.MenuSubButton href="/repositories" isActive={repositoriesActive} aria-current={repositoriesActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                <BookOpen aria-hidden="true" /><span>Repositories</span>
              </Sidebar.MenuSubButton>
            </Sidebar.MenuSubItem>
            <Sidebar.MenuSubItem>
              <Sidebar.MenuSubButton href="/source-control" isActive={sourceControlActive} aria-current={sourceControlActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                <GitBranch aria-hidden="true" /><span>Source control</span>
              </Sidebar.MenuSubButton>
              <Sidebar.MenuSub class="mx-2 px-1">
                <Sidebar.MenuSubItem>
                  <Sidebar.MenuSubButton href="/source-control/github" isActive={githubActive} aria-current={githubActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                    <Github aria-hidden="true" /><span>GitHub</span>
                  </Sidebar.MenuSubButton>
                </Sidebar.MenuSubItem>
              </Sidebar.MenuSub>
            </Sidebar.MenuSubItem>
          </SidebarNavGroup>
          <SidebarNavGroup title="Runtimes">
            {#snippet icon()}<Layers aria-hidden="true" />{/snippet}
            <SidebarNavGroup title="Ayni">
              {#snippet icon()}<Wrench aria-hidden="true" />{/snippet}
              <Sidebar.MenuSubItem>
                <Sidebar.MenuSubButton href="/certificates" isActive={certificatesActive} aria-current={certificatesActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                  <ShieldCheck aria-hidden="true" /><span>Certificates</span>
                </Sidebar.MenuSubButton>
              </Sidebar.MenuSubItem>
              <Sidebar.MenuSubItem>
                <Sidebar.MenuSubButton href="/container-registries" isActive={registriesActive} aria-current={registriesActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                  <Container aria-hidden="true" /><span>Container registries</span>
                </Sidebar.MenuSubButton>
              </Sidebar.MenuSubItem>
            </SidebarNavGroup>
          </SidebarNavGroup>
        </Sidebar.Menu>
      </Sidebar.GroupContent>
    </Sidebar.Group>
  </Sidebar.Content>
  <Sidebar.Rail />
</Sidebar.Root>
