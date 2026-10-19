<script lang="ts">
import RegistryConnectionManager from "$lib/components/registry-connection-manager.svelte";
import type { PageProps } from "./$types";
let {data,form}:PageProps=$props();
</script>
<svelte:head><title>GitHub Container Registry · Ayni Orchestrator</title></svelte:head>
<div class="max-w-3xl space-y-6"><h1 class="text-3xl font-semibold">GitHub Container Registry</h1><p>Manage credentials for repository testing environments.</p><RegistryConnectionManager {...data}/></div>
