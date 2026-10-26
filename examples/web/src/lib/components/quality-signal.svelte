<script lang="ts">
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";

  let { row }: { row: { kind: string; passed: boolean; status?: "pass" | "warn" | "fail"; summary?: string; target?: string } } = $props();
  const status = $derived(row.status ?? (row.passed ? "pass" : "fail"));
  const summary = $derived(row.summary || "A summary was not recorded for this check.");
  const label = $derived(`${row.kind}: ${status}${row.target ? ` · ${row.target}` : ""}`);
</script>

<Tooltip.Provider delayDuration={150}>
  <Tooltip.Root>
    <Tooltip.Trigger class="flex items-center gap-2 rounded-lg border px-3 py-2 text-sm hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={label}>
      <img src={`/icons/ayni/${status}.svg`} alt={status} width="20" height="20" />
      <span>{row.kind}</span>
    </Tooltip.Trigger>
    <Tooltip.Content role="tooltip" class="block max-w-sm space-y-1 break-words">
      {#if row.target}<p class="font-medium">{row.target}</p>{/if}
      <p>{summary}</p>
    </Tooltip.Content>
  </Tooltip.Root>
</Tooltip.Provider>
