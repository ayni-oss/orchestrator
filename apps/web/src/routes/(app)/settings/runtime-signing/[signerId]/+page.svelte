<script lang="ts">
  import RuntimeSignerManager from '$lib/components/runtime-signer-manager.svelte';

  let { params } = $props();
</script>

<svelte:head><title>Signing key settings · Ayni Orchestrator</title></svelte:head>

{#key params.signerId}
  <RuntimeSignerManager signerId={params.signerId} />
{/key}
