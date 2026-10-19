<script lang="ts">
  import type { PageProps } from "./$types";
  import PageHeader from "$lib/components/page-header.svelte";
  import CertificateManager from "$lib/components/certificate-manager.svelte";
  let { data }: PageProps = $props();
</script>
<svelte:head><title>Certificates · Ayni Orchestrator</title></svelte:head>
<div class="mx-auto max-w-5xl space-y-6">
  <PageHeader flush title="Certificates" description="Manage signing certificates for Ayni testing environment images. Choose a certificate in each repository’s runtime settings; projects inherit that selection." />

  <CertificateManager certificates={data.certificates} available={data.available} idempotencyKey={data.idempotencyKey} />
</div>
