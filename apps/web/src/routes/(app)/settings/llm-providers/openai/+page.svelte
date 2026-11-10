<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
 import PageHeader from '$lib/components/page-header.svelte';
 import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';
</script>
<PageHeader eyebrow="LLM providers" title="OpenAI" />
<PageContent>
<ProviderConnectionManager capability="llm" />
</PageContent>
