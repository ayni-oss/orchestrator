<script lang="ts">
  import { Moon, Sun } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { Button } from "$lib/components/ui/button/index.js";

  let dark = $state(false);
  onMount(() => {
    const saved = localStorage.getItem("ayni-theme");
    dark = saved === "dark" || (saved === null && matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.classList.toggle("dark", dark);
  });
  function toggle(): void {
    dark = !dark;
    document.documentElement.classList.toggle("dark", dark);
    localStorage.setItem("ayni-theme", dark ? "dark" : "light");
  }
</script>

<Button variant="ghost" size="icon" onclick={toggle} aria-label="Toggle color theme" title="Toggle color theme">
  {#if dark}<Moon class="size-4" aria-hidden="true" />{:else}<Sun class="size-4" aria-hidden="true" />{/if}
</Button>
