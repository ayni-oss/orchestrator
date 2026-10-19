<script lang="ts">
import PageHeader from "$lib/components/page-header.svelte";
import RegistryConnectionManager from "$lib/components/registry-connection-manager.svelte";
import type { PageProps } from "./$types";
let {data,form}:PageProps=$props();
</script>
<svelte:head><title>GitHub Container Registry · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-5xl space-y-6"><PageHeader flush eyebrow="Container registries" title="GitHub Container Registry" description="Manage credentials for repository testing environments." /><RegistryConnectionManager {...data}/></div>
