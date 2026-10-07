<script lang="ts">
import ConnectionManager from "$lib/components/connection-manager.svelte";
import type { PageProps } from "./$types";
let {data,form}:PageProps=$props();
</script>
<svelte:head><title>GitHub Container Registry · Ayni Orchestrator</title></svelte:head>
<div class="max-w-3xl space-y-6"><h1 class="text-3xl font-semibold">GitHub Container Registry</h1><p>Manage credentials for repository testing environments.</p>{#if form?.error || data.error}<p role="alert">{(form?.error ?? data.error).message}</p>{/if}{#if form?.success}<p role="status">{form.success}</p>{/if}<ConnectionManager {...data} kind="ghcr"/></div>
