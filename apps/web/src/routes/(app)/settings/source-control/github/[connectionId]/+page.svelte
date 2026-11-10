<script lang="ts">
  import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';

  let { params } = $props();
</script>

<svelte:head><title>GitHub App settings · ayni Orchestrator</title></svelte:head>

{#key params.connectionId}
  <ProviderConnectionManager capability="source_control" connectionId={params.connectionId} />
{/key}
