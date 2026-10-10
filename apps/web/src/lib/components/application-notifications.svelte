<script lang="ts">
  import { page } from "$app/state";
  import { Toaster } from "$lib/components/ui/sonner/index.js";
  import { createNotificationEvents } from "$lib/notifications.js";

  const events = createNotificationEvents();
  $effect(() => events.action(page.form));
  $effect(() => events.load(page.url.pathname, page.data));
</script>

<Toaster position="bottom-right" closeButton />
