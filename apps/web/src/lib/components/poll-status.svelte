<script lang="ts">
  import { onMount } from "svelte";
  import { subscribeRefresh, type RefreshScope } from "$lib/refresh.js";
  let { scope = "app:runtimes" }: { scope?: RefreshScope } = $props();
  let message=$state("Live updates every 10 seconds; paused while editing");
  onMount(() => subscribeRefresh(scope, {intervalMs:10_000,onResult:ok => {
    message=ok ? "Live updates every 10 seconds; paused while editing" : "Updates interrupted — retrying automatically";
  }}));
</script>
<p class="text-xs text-muted-foreground" role="status">{message}</p>
