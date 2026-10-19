<script lang="ts">
  import { resourceChanged } from "$lib/refresh.js";
  import { onMount } from "svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";

  let { taskId, resourceVersion }: { taskId: string; resourceVersion: string } = $props();
  let connection = $state<"connecting" | "live" | "stale" | "disconnected">("connecting");
  let lastUpdate = $state(Date.now());
  let now = $state(Date.now());
  let source: EventSource | undefined;
  $effect(() => { if (connection === "live" && now - lastUpdate > 45_000) connection = "stale"; });

  onMount(() => {
    const interval = window.setInterval(() => { now = Date.now(); }, 5_000);
    let retry: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    function connect() {
    if (stopped) return;
    source = new EventSource(`/tasks/${taskId}/updates`);
    const stream = source;
    stream.onopen = () => { connection = "live"; lastUpdate = Date.now(); };
    stream.addEventListener("heartbeat", () => {
      lastUpdate = Date.now();
      connection = "live";
    });
    stream.addEventListener("task", (event) => {
      lastUpdate = Date.now();
      connection = "live";
      try {
        const update = JSON.parse((event as MessageEvent<string>).data) as { resourceVersion?: string };
        if (update.resourceVersion !== undefined && update.resourceVersion !== resourceVersion) resourceChanged("app:tasks");
      } catch { connection = "disconnected"; }
    });
    stream.addEventListener("related", () => {
      lastUpdate = Date.now();
      resourceChanged("app:tasks");
    });
    stream.addEventListener("error", event => {
      connection = "disconnected";
      if (event instanceof MessageEvent) {
        try {
          const error = JSON.parse(event.data) as { reason?: string };
          if (error.reason === "watch_cursor_expired") {
            stream.close();
            resourceChanged("app:tasks");
            retry = setTimeout(connect, 1_000);
          }
        } catch { /* Native errors reconnect using Last-Event-ID. */ }
      }
    });
    }
    connect();
    return () => {
      stopped = true;
      clearTimeout(retry);
      window.clearInterval(interval);
      source?.close();
      source = undefined;
    };
  });
</script>

<div class="flex items-center gap-2" role="status" aria-live="polite">
  <span class="text-xs text-muted-foreground">Updates</span>
  <StatusBadge status={connection} tone={connection === "live" ? "success" : connection === "connecting" ? "active" : connection === "stale" ? "warning" : "danger"} pulse={connection === "live" || connection === "connecting"} />
</div>
