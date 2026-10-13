<script lang="ts">
  import { onMount } from "svelte";
  import { subscribeRefresh } from "$lib/refresh.js";
  import { page } from "$app/state";
  import { Toaster } from "$lib/components/ui/sonner/index.js";
  import { createNotificationEvents } from "$lib/notifications.js";

  onMount(() => {
    const projects = subscribeRefresh("app:projects", { intervalMs: 120_000 });
    const repositories = subscribeRefresh("app:repositories", { intervalMs: 120_000 });
    const runtimes = subscribeRefresh("app:runtimes", { intervalMs: 120_000 });
    const tasks = subscribeRefresh("app:tasks", { intervalMs: 30_000 });
    return () => { projects(); repositories(); runtimes(); tasks(); };
  });
  const events = createNotificationEvents();
  $effect(() => events.action(page.form));
  $effect(() => events.load(page.url.pathname, page.data));
</script>

<Toaster position="bottom-right" closeButton />
