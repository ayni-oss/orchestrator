<script lang="ts">
  import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';

  let { params } = $props();
</script>

<svelte:head><title>OpenAI settings · ayni Orchestrator</title></svelte:head>

{#key params.connectionId}
  <ProviderConnectionManager capability="llm" connectionId={params.connectionId} />
{/key}
