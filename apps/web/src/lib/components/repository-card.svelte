<script lang="ts">
 import { repositoryPath } from "$lib/paths";
 import { goto } from '$app/navigation';
 import InformationCard from './information-card.svelte';
 import type { Repository } from '$lib/repositories';
 let { repo }: { repo: Repository } = $props();
 const href = $derived(repositoryPath(repo.id));
</script>
<InformationCard title={repo.fullName} status="connected" headingLevel={2} navigationHref={href} actions={[
 {label:'ayni',onSelect:()=>{void goto(`${href}?tab=runtime`);}},
 {label:'Source integration details',onSelect:()=>{void goto(`${href}?details=source`);}},
]}>
 <p class="min-h-16 text-sm leading-relaxed text-muted-foreground">{repo.private ? 'Private repository' : 'Public repository'}<br/>Default branch: {repo.defaultBranch}</p>
</InformationCard>
