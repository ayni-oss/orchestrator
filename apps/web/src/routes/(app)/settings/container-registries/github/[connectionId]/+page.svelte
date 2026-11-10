<script lang="ts">
  import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';

  let { params } = $props();
</script>

<svelte:head><title>Registry settings · Ayni Orchestrator</title></svelte:head>

{#key params.connectionId}
  <ProviderConnectionManager capability="container_registry" connectionId={params.connectionId} />
{/key}
