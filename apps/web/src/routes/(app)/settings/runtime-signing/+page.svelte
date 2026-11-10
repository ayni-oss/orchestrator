<script lang="ts">
 import PageContent from "$lib/components/page-content.svelte";
 import PageHeader from '$lib/components/page-header.svelte';
 import RuntimeSignerManager from '$lib/components/runtime-signer-manager.svelte';
</script>

<PageHeader title="Runtime signing"/>
<PageContent>
 <p class="text-sm text-muted-foreground">Create or import a signing identity, then commit its public key to the repository trust policy.</p>
 <RuntimeSignerManager />
</PageContent>
