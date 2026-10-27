<script lang="ts">
 import PageHeader from '$lib/components/page-header.svelte';
 import ProviderConnectionManager from '$lib/components/provider-connection-manager.svelte';
</script>
<div class="mx-auto max-w-5xl space-y-8"><PageHeader flush eyebrow="LLM providers" title="OpenAI" description="Manage API connections for workspace conversations." />
<ProviderConnectionManager capability="llm" />
</div>
