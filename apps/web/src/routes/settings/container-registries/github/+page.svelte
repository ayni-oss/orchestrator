<script lang="ts">
 import PageHeader from '$lib/components/page-header.svelte';
 import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';
</script>
<div class="mx-auto max-w-5xl space-y-8"><PageHeader flush eyebrow="Container registries" title="GitHub" description="Connect to GitHub Container Registry with separate push and pull access." />
<ProviderConnectionManager capability="container_registry" />
</div>
