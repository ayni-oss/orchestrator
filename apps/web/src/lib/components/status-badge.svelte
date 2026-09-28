<script lang="ts">
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { words, type StatusTone } from "$lib/presentation.js";
  let { status, tone = "neutral", pulse = false }: { status: string; tone?: StatusTone; pulse?: boolean } = $props();
  const toneClass: Record<StatusTone, string> = {
    neutral: "border-border bg-muted text-muted-foreground",
    active: "border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300",
    success: "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    warning: "border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-300",
    danger: "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-300",
  };
</script>

<Badge variant="outline" class={toneClass[tone]}>
  {#if pulse}<span class="mr-1 size-1.5 rounded-full bg-current motion-safe:animate-pulse" aria-hidden="true"></span>{/if}
  {words(status)}
</Badge>
