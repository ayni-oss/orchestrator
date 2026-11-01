<script lang="ts">
 import { page } from '$app/state';
 import RepositoryDetail from '$lib/components/repository-detail.svelte';
</script>
{#key page.params.id}<RepositoryDetail repositoryId={page.params.id!}/>{/key}
