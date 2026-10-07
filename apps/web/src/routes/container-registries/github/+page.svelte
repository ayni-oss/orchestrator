<script lang="ts">
  import { Container } from "@lucide/svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
</script>

<svelte:head><title>GitHub Container Registry · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8">
  <PageHeader title="GitHub Container Registry" description="Store repository testing environments in GitHub." />
  <Card.Root class="max-w-3xl">
    <Card.Header><Container class="mb-3 size-6" aria-hidden="true" /><Card.Title>Registry connection</Card.Title><Card.Description>Registry connections are currently managed by deployment configuration. Credential onboarding and connection tests are not available yet.</Card.Description></Card.Header>
    <Card.Content><p class="text-sm text-muted-foreground">One shared image destination per repository.</p></Card.Content>
    <Card.Footer><Button href="/container-registries" variant="outline">Back to container registries</Button></Card.Footer>
  </Card.Root>
</div>
