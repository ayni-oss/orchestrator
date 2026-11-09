<script lang="ts">
 import InformationCard from '$lib/components/information-card.svelte';
 import { projectActions, workspaceActions } from '$lib/hierarchy-actions';
 let { kind, id, name, resourceVersion, workspaceId, description='', href }: {
  kind:'workspace'|'project';id:string;name:string;resourceVersion:string;workspaceId?:string;description?:string;href:string;
 }=$props();
</script>
<InformationCard title={name} headingLevel={2} navigationHref={href} actions={kind === 'workspace' ? workspaceActions(id) : projectActions(id, workspaceId)}><p class="min-h-16 text-sm leading-relaxed text-muted-foreground">{description|| (kind==='workspace'?'Open workspace projects.':'No project goal yet.')}</p></InformationCard>
