<script lang="ts">
  import { onMount } from "svelte";
  import { CircleCheck, Info, LoaderCircle, OctagonX, TriangleAlert } from "@lucide/svelte";
  import { Toaster as Sonner, type ToasterProps } from "svelte-sonner";
  let { ...props }: ToasterProps = $props();
  let theme = $state<"light" | "dark">("light");
  onMount(() => {
    const update = () => { theme = document.documentElement.classList.contains("dark") ? "dark" : "light"; };
    update();
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  });
</script>

<Sonner {theme} class="toaster group" style="--normal-bg: var(--color-popover); --normal-text: var(--color-popover-foreground); --normal-border: var(--color-border);" {...props}>
  {#snippet loadingIcon()}<LoaderCircle class="size-4 animate-spin" />{/snippet}
  {#snippet successIcon()}<CircleCheck class="size-4" />{/snippet}
  {#snippet errorIcon()}<OctagonX class="size-4" />{/snippet}
  {#snippet infoIcon()}<Info class="size-4" />{/snippet}
  {#snippet warningIcon()}<TriangleAlert class="size-4" />{/snippet}
</Sonner>
