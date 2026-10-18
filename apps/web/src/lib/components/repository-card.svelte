<script lang="ts">
  import type { RepositorySummary } from "$lib/server/view-types.js";
  import AyniIcon from "$lib/components/ayni-icon.svelte";
  import InformationCard from "$lib/components/information-card.svelte";
  import InformationTable from "$lib/components/information-table.svelte";

  let { summary }: { summary: RepositorySummary } = $props();
  const item = $derived(summary.repository);
  const readiness = $derived(!summary.runtime.complete ? "Unavailable" : summary.runtime.ready ? "Ready" : item.useAyni === false ? "Ayni not enabled" : "Setup required");
  const repositoryHref = $derived(`/_old/repositories/${encodeURIComponent(item.id)}`);
</script>

<a href={repositoryHref} aria-label={`Open repository ${item.displayName}`} class="block min-w-0 rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background [&>div]:h-full [&>div]:transition-colors hover:[&>div]:bg-accent/30">
  <InformationCard title={item.displayName} status={readiness} tone={summary.runtime.ready ? "success" : "warning"}>
    {#snippet titleAccessory()}
      {#if item.useAyni !== false}<AyniIcon class="size-5 shrink-0 text-muted-foreground" role="img" aria-label="Ayni enabled" />{/if}
    {/snippet}
    <InformationTable rows={[
      { label: "Default branch", value: item.defaultBranch },
      { label: "Runtimes", value: summary.runtime.complete ? `${summary.runtime.readyProfiles} of ${summary.runtime.operationalProfiles} operational runtimes ready` : "Unavailable" },
    ]} />
  </InformationCard>
</a>
