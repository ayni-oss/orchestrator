<script lang="ts">
  import { FolderGit2, BookOpen, GitBranch, Container, Settings, BrainCircuit } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { subscribeRefresh } from "$lib/refresh.js";
  import AyniIcon from "$lib/components/ayni-icon.svelte";
  import { page } from "$app/state";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import SidebarStatus from "$lib/components/sidebar-status.svelte";
  import SidebarNavGroup from "$lib/components/sidebar-nav-group.svelte";

  const sidebar = Sidebar.useSidebar();
  const repositoriesActive = $derived(page.url.pathname.startsWith("/repositories") || page.url.pathname.startsWith("/profiles"));
  const sourceControlActive = $derived(page.url.pathname.startsWith("/source-control") || page.url.pathname.startsWith("/setup/github"));
  const registriesActive = $derived(page.url.pathname.startsWith("/container-registries"));
  const ayniActive = $derived(page.url.pathname.startsWith("/ayni") || page.url.pathname.startsWith("/certificates"));
  const workspacesActive = $derived(page.url.pathname.startsWith("/workspaces") || page.url.pathname.startsWith("/projects") || page.url.pathname.startsWith("/tasks"));
  const sourceControlConfiguration = $derived(page.data.sourceControlConfiguration);
  const registryConfiguration = $derived(page.data.registryConfiguration);
  const certificateConfiguration = $derived(page.data.certificateConfiguration);
  onMount(() => {
    const workspaces = subscribeRefresh("app:workspaces", { intervalMs: 30_000 });
    const certificates = subscribeRefresh("app:certificates", { intervalMs: 30_000 });
    const connections = subscribeRefresh("app:connections", { intervalMs: 30_000 });
    return () => { workspaces(); certificates(); connections(); };
  });
</script>

<Sidebar.Root collapsible="icon" class="shrink-0">
  <Sidebar.Header>
    <a onclick={() => sidebar.setOpenMobile(false)} href="/workspaces" aria-label="Ayni Orchestrator home" class="flex h-10 items-center gap-2 px-2 font-semibold no-underline">
      <AyniIcon variant="orchestrator" class="size-5 shrink-0" aria-hidden="true" />
      <span class="group-data-[collapsible=icon]:hidden">Ayni Orchestrator</span>
    </a>
  </Sidebar.Header>
  <Sidebar.Content>
    <Sidebar.Group>
      <Sidebar.GroupContent>
        <Sidebar.Menu>
          <Sidebar.MenuItem><Sidebar.MenuButton isActive={workspacesActive} tooltipContent="Workspaces">{#snippet child({props})}<a {...props} onclick={()=>sidebar.setOpenMobile(false)} href="/workspaces" aria-current={workspacesActive?"page":undefined}><FolderGit2 aria-hidden="true" /><span>Workspaces</span></a>{/snippet}</Sidebar.MenuButton></Sidebar.MenuItem>
          <Sidebar.MenuItem>
            <Sidebar.MenuButton isActive={repositoriesActive} tooltipContent="Repositories">
              {#snippet child({ props })}
                <a {...props} onclick={() => sidebar.setOpenMobile(false)} href="/repositories" aria-current={repositoriesActive ? "page" : undefined}>
                  <BookOpen aria-hidden="true" /><span>Repositories</span>
                </a>
              {/snippet}
            </Sidebar.MenuButton>
          </Sidebar.MenuItem>
          <SidebarNavGroup title="Config">
            {#snippet icon()}<Settings aria-hidden="true" />{/snippet}
            <Sidebar.MenuSubItem>
              <Sidebar.MenuSubButton class="translate-x-0" href="/source-control" isActive={sourceControlActive} aria-current={sourceControlActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                <GitBranch aria-hidden="true" /><span>Source control</span>
                <SidebarStatus status={sourceControlConfiguration ?? { status: "warning", description: "Source control configuration status is unavailable." }} />
              </Sidebar.MenuSubButton>
            </Sidebar.MenuSubItem>
            <Sidebar.MenuSubItem>
              <Sidebar.MenuSubButton class="translate-x-0" href="/container-registries" isActive={registriesActive} aria-current={registriesActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                <Container aria-hidden="true" /><span>Container registries</span>
                <SidebarStatus status={registryConfiguration ?? { status: "warning", description: "Container registry permission status is unavailable." }} />
              </Sidebar.MenuSubButton>
            </Sidebar.MenuSubItem>
            <Sidebar.MenuSubItem>
              <Sidebar.MenuSubButton class="translate-x-0" href="/llm-providers" isActive={page.url.pathname.startsWith("/llm-providers")} onclick={() => sidebar.setOpenMobile(false)}>
                <BrainCircuit aria-hidden="true" /><span>LLM providers</span>
                <SidebarStatus status={page.data.llmConfiguration ?? {status:"warning",description:"LLM provider status is unavailable."}} />
              </Sidebar.MenuSubButton>
            </Sidebar.MenuSubItem>
            <Sidebar.MenuSubItem>
              <Sidebar.MenuSubButton class="translate-x-0" href="/ayni" isActive={ayniActive} aria-current={ayniActive ? "page" : undefined} onclick={() => sidebar.setOpenMobile(false)}>
                <AyniIcon aria-hidden="true" /><span>Ayni</span>
                <SidebarStatus status={certificateConfiguration ?? { status: "warning", description: "Certificate configuration status is unavailable." }} />
              </Sidebar.MenuSubButton>
            </Sidebar.MenuSubItem>
          </SidebarNavGroup>
        </Sidebar.Menu>
      </Sidebar.GroupContent>
    </Sidebar.Group>
  </Sidebar.Content>
  <Sidebar.Rail />
</Sidebar.Root>
