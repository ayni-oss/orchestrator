<script lang="ts">
  import { onMount } from "svelte";
  import type { LayoutProps } from "./$types";
  import ApplicationNotifications from "$lib/components/application-notifications.svelte";
  import "../app.css";
  let { children }: LayoutProps = $props();
  onMount(() => { document.documentElement.dataset.hydrated = "true"; });
</script>

<ApplicationNotifications />
{@render children()}
