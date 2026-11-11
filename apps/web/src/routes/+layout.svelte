<script lang="ts">
 import '../app.css';
 import DevStatus from '$lib/DevStatus.svelte';
 let { children } = $props();
</script>
{@render children()}
{#if import.meta.env.DEV}<DevStatus />{/if}
