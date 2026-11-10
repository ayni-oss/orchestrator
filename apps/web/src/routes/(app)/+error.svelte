<script lang="ts">
 import { page } from '$app/state';
 import PageHeader from '$lib/components/page-header.svelte';
 import PageContent from '$lib/components/page-content.svelte';
 import { Button } from '$lib/components/ui/button';
</script>
<PageHeader title={page.status === 404 ? 'Page not found' : 'Page unavailable'} />
<PageContent>
 <p role="alert">{page.error?.message}</p>
 <div class="flex gap-3"><Button href="/">Workspaces</Button><Button variant="outline" onclick={() => location.reload()}>Try again</Button></div>
</PageContent>
