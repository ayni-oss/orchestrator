<script lang="ts">
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import type { LayoutProps } from "./$types";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import "../app.css";
  let { children }: LayoutProps = $props();
  const sourceControl = $derived(page.url.pathname.startsWith("/source-control") || page.url.pathname.startsWith("/setup/github"));
  const registries = $derived(page.url.pathname.startsWith("/container-registries"));
  const repositories = $derived(page.url.pathname.startsWith("/repositories") || page.url.pathname.startsWith("/profiles"));
  const current = $derived(page.url.pathname.startsWith("/tasks") ? "Task" : page.url.pathname.startsWith("/profiles") ? "Runtime" : page.url.pathname.startsWith("/repositories/") ? "Repository" : (page.url.pathname.startsWith("/setup/github/complete") || page.url.pathname.startsWith("/source-control/github/complete")) ? "GitHub App" : (page.url.pathname.startsWith("/setup/github") || page.url.pathname.startsWith("/source-control/github")) ? "GitHub" : page.url.pathname.startsWith("/container-registries/github") ? "GitHub" : page.url.pathname.startsWith("/projects/") ? "Project" : "");
  onMount(() => { document.documentElement.dataset.hydrated = "true"; });
</script>
<a href="#main-content" class="fixed left-3 top-3 z-50 -translate-y-20 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground focus:translate-y-0">Skip to content</a>
<Sidebar.Provider><AppSidebar /><Sidebar.Inset class="min-w-0" id="main-content" tabindex={-1}>
  <header class="sticky top-0 z-20 flex h-16 items-center gap-3 border-b bg-background/95 px-4 backdrop-blur sm:px-6">
    <Sidebar.Trigger /><nav aria-label="Breadcrumb" class="flex-1 text-sm"><a class="text-muted-foreground hover:text-foreground" href={sourceControl ? "/source-control" : registries ? "/container-registries" : repositories ? "/repositories" : "/projects"}>{sourceControl ? "Source control" : registries ? "Container registries" : repositories ? "Repositories" : "Projects"}</a>{#if current}<span aria-hidden="true" class="mx-2 text-muted-foreground">/</span><span aria-current="page">{current}</span>{/if}</nav><ThemeToggle />
  </header>
  <div class="min-w-0 flex-1 space-y-8 p-4 sm:p-6 lg:p-8">{@render children()}</div>
</Sidebar.Inset></Sidebar.Provider>
