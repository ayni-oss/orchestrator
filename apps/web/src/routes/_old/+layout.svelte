<script lang="ts">
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import type { LayoutProps } from "./$types";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  let { children }: LayoutProps = $props();
  const workspaces = $derived(page.url.pathname.startsWith("/_old/workspaces"));
  const sourceControl = $derived(page.url.pathname.startsWith("/_old/source-control") || page.url.pathname.startsWith("/_old/setup/github"));
  const registries = $derived(page.url.pathname.startsWith("/_old/container-registries"));
  const certificates = $derived(page.url.pathname.startsWith("/_old/certificates"));
  const ayni = $derived(page.url.pathname.startsWith("/_old/ayni") || certificates);
  const repositories = $derived(page.url.pathname.startsWith("/_old/repositories") || page.url.pathname.startsWith("/_old/profiles"));
  const current = $derived(workspaces && page.url.pathname.startsWith("/_old/workspaces/") ? "Workspace" : certificates ? "Certificates" : page.url.pathname.startsWith("/_old/tasks") ? "Task" : page.url.pathname.startsWith("/_old/profiles") ? "Runtime" : page.url.pathname.startsWith("/_old/repositories/") ? "Repository" : (page.url.pathname.startsWith("/_old/setup/github/complete") || page.url.pathname.startsWith("/_old/source-control/github/complete")) ? "GitHub App" : (page.url.pathname.startsWith("/_old/setup/github") || page.url.pathname.startsWith("/_old/source-control/github")) ? "GitHub" : page.url.pathname.startsWith("/_old/container-registries/github") ? "GitHub" : page.url.pathname.startsWith("/_old/projects/") ? "Project" : "");
  onMount(() => { document.documentElement.dataset.hydrated = "true"; });
</script>
<a href="#main-content" class="fixed left-3 top-3 z-50 -translate-y-20 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground focus:translate-y-0">Skip to content</a>
<ApplicationNotifications />
<Sidebar.Provider><AppSidebar /><Sidebar.Inset class="min-w-0" id="main-content" tabindex={-1}>
  <header class="sticky top-0 z-20 flex h-16 items-center gap-3 border-b bg-background/95 px-4 backdrop-blur sm:px-6">
    <Sidebar.Trigger /><nav aria-label="Breadcrumb" class="flex-1 text-sm">{#if page.data.workspace}
      <a class="text-muted-foreground hover:text-foreground" href="/_old/workspaces">Workspaces</a><span aria-hidden="true" class="mx-2 text-muted-foreground">/</span>
      {#if page.data.project}<a class="text-muted-foreground hover:text-foreground" href={`/_old/workspaces/${encodeURIComponent(page.data.workspace.id)}`}>{page.data.workspace.displayName}</a><span aria-hidden="true" class="mx-2 text-muted-foreground">/</span><span aria-current="page">{page.data.project.displayName}</span>
      {:else}<span aria-current="page">{page.data.workspace.displayName}</span>{/if}
    {:else}<a class="text-muted-foreground hover:text-foreground" href={workspaces ? "/_old/workspaces" : ayni ? "/_old/ayni" : sourceControl ? "/_old/source-control" : registries ? "/_old/container-registries" : repositories ? "/_old/repositories" : "/_old/workspaces"}>{workspaces ? "Workspaces" : ayni ? "Ayni" : sourceControl ? "Source control" : registries ? "Container registries" : repositories ? "Repositories" : "Workspaces"}</a>{#if current}<span aria-hidden="true" class="mx-2 text-muted-foreground">/</span><span aria-current="page">{current}</span>{/if}{/if}</nav><ThemeToggle />
  </header>
  <div class="min-w-0 flex-1 space-y-8 p-4 sm:p-6 lg:p-8">{@render children()}</div>
</Sidebar.Inset></Sidebar.Provider>
