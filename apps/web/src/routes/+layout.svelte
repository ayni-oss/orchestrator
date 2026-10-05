<script lang="ts">
  import { Boxes, FolderGit2, ListTodo, PlugZap } from "@lucide/svelte";
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import type { LayoutProps } from "./$types";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import "../app.css";

  let { children }: LayoutProps = $props();
  const links = [
    { href: "/projects", label: "Projects", icon: FolderGit2 },
    { href: "/tasks", label: "Tasks", icon: ListTodo },
    { href: "/setup/github", label: "Connectors", icon: PlugZap },
  ];
  const active = (href: string): boolean => href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href);
  onMount(() => { document.documentElement.dataset.hydrated = "true"; });
</script>

<a href="#main-content" class="fixed left-3 top-3 z-50 -translate-y-20 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground focus:translate-y-0">Skip to content</a>
<div class="min-h-screen bg-background">
  <header class="sticky top-0 z-40 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/80">
    <div class="mx-auto flex h-16 max-w-7xl items-center gap-4 px-4 sm:px-6 lg:px-8">
      <a href="/" class="flex shrink-0 items-center gap-2 font-semibold no-underline" aria-label="Ayni Orchestrator overview">
        <span class="grid size-8 place-items-center rounded-lg bg-primary text-primary-foreground" aria-hidden="true"><Boxes class="size-4" /></span>
        <span class="hidden sm:inline">Ayni Orchestrator</span>
      </a>
      <nav aria-label="Primary" class="min-w-0 flex-1 overflow-x-auto">
        <ul class="flex w-max items-center gap-1">
          {#each links as link}
            <li>
              <a href={link.href} aria-current={active(link.href) ? "page" : undefined} class:font-semibold={active(link.href)} class:bg-accent={active(link.href)} class="flex items-center gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground no-underline hover:bg-accent hover:text-foreground aria-[current=page]:text-foreground">
                <link.icon class="size-4" aria-hidden="true" />
                {link.label}
              </a>
            </li>
          {/each}
        </ul>
      </nav>
      <ThemeToggle />
    </div>
  </header>
  <main id="main-content" class="mx-auto max-w-7xl space-y-8 px-4 py-8 sm:px-6 lg:px-8" tabindex="-1">
    {@render children()}
  </main>
  <footer class="mx-auto max-w-7xl border-t px-4 py-6 text-xs text-muted-foreground sm:px-6 lg:px-8">
    Trusted local operator interface · Product state is authoritative in the control plane.
  </footer>
</div>
