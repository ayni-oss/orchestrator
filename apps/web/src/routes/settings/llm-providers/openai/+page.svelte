<script lang="ts">
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import LlmConnectionManager from "$lib/components/llm-connection-manager.svelte";
  let {data}:PageProps=$props();
</script>
<svelte:head><title>OpenAI · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-5xl space-y-8"><PageHeader flush title="OpenAI" description="Connect OpenAI models for project planning and coding." /><LlmConnectionManager connections={data.connections} idempotencyKey={data.idempotencyKey} available={data.available} /></div>
