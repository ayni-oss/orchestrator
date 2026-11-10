<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
 import PageHeader from '$lib/components/page-header.svelte';
 import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';
</script>
<PageHeader eyebrow="Container registries" title="GitHub" />
<PageContent>
<ProviderConnectionManager capability="container_registry" />
</PageContent>
