<script lang="ts">
  import { invalidateAll } from "$app/navigation";
  import { onMount } from "svelte";
  import StatusBadge from "$lib/components/status-badge.svelte";

  let { taskId, resourceVersion, terminal }: { taskId: string; resourceVersion: string; terminal: boolean } = $props();
  let connection = $state<"connecting" | "live" | "stale" | "disconnected" | "retained">("connecting");
  let lastUpdate = $state(Date.now());
  let now = $state(Date.now());
  let source: EventSource | undefined;
  $effect(() => {
    if (terminal) {
      connection = "retained";
      source?.close();
      source = undefined;
    }
  });
  $effect(() => { if (!terminal && connection === "live" && now - lastUpdate > 45_000) connection = "stale"; });

  onMount(() => {
    if (terminal) return;
    const interval = window.setInterval(() => { now = Date.now(); }, 5_000);
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
        if (update.resourceVersion !== undefined && update.resourceVersion !== resourceVersion) void invalidateAll();
      } catch { connection = "disconnected"; }
    });
    stream.addEventListener("error", () => { if (!terminal) connection = "disconnected"; });
    return () => {
      window.clearInterval(interval);
      stream.close();
      if (source === stream) source = undefined;
    };
  });
</script>

<div class="flex items-center gap-2" role="status" aria-live="polite">
  <span class="text-xs text-muted-foreground">Updates</span>
  <StatusBadge status={connection} tone={connection === "live" || connection === "retained" ? "success" : connection === "connecting" ? "active" : connection === "stale" ? "warning" : "danger"} pulse={connection === "live" || connection === "connecting"} />
</div>
