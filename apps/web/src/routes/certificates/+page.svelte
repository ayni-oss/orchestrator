<script lang="ts">
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import Feedback from "$lib/components/feedback.svelte";
  import CertificateManager from "$lib/components/certificate-manager.svelte";
  let { data, form }: PageProps = $props();
</script>
<svelte:head><title>Certificates · Ayni Orchestrator</title></svelte:head>
<div class="max-w-4xl space-y-6">
  <PageHeader title="Certificates" description="Manage signing certificates for Ayni testing environment images. Choose a certificate in each repository’s runtime settings; projects inherit that selection." />
  {#if data.error}<Feedback kind="error" title="Certificates unavailable" message={data.error.message} />{/if}
  <CertificateManager certificates={data.certificates} available={data.available} idempotencyKey={data.idempotencyKey} error={form?.error?.message} />
</div>
