<script lang="ts">
 import PageHeader from '$lib/components/page-header.svelte';
 import RuntimeSignerManager from '$lib/components/runtime-signer-manager.svelte';
</script>

<PageHeader title="Runtime signing" description="Create or import a signing identity, then commit its public key to the repository trust policy."/>
<RuntimeSignerManager />
