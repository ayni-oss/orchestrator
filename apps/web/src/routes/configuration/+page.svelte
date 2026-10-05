<script lang="ts">
  import { GitBranch, Container } from "@lucide/svelte";
  import PageHeader from "$lib/components/page-header.svelte";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
</script>
<svelte:head><title>Configuration · Ayni Orchestrator</title></svelte:head>
<div class="space-y-8"><PageHeader title="Configuration" description="Connect source control and configure where testing environments are stored." />
<div class="grid gap-5 md:grid-cols-2">
<Card.Root><Card.Header><GitBranch class="mb-3 size-6" aria-hidden="true" /><Card.Title>Source control</Card.Title><Card.Description>Connect a customer-owned GitHub App and choose accessible repositories.</Card.Description></Card.Header><Card.Content class="flex flex-wrap gap-3"><Button href="/setup/github">Configure GitHub</Button><Button href="/repositories" variant="outline">Repository connections</Button></Card.Content></Card.Root>
<Card.Root><Card.Header><Container class="mb-3 size-6" aria-hidden="true" /><Card.Title>Container registries</Card.Title><Card.Description>Registry connections are currently managed by deployment configuration. Credential onboarding and connection tests are not available yet.</Card.Description></Card.Header><Card.Content><p class="text-sm text-muted-foreground">One shared image destination per repository.</p></Card.Content></Card.Root>
</div></div>
