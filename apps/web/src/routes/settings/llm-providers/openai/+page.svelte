<script lang="ts">
 import PageHeader from '$lib/components/page-header.svelte';
 import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';
</script>
<PageHeader eyebrow="LLM providers" title="OpenAI" />
<div class="mx-auto max-w-5xl space-y-8">
<ProviderConnectionManager capability="llm" />
</div>
