<script lang="ts">
 import InformationCard from '$lib/components/information-card.svelte';
 import RenameHierarchyDialog from '$lib/components/rename-hierarchy-dialog.svelte';
 import { hierarchyActions } from '$lib/hierarchy-actions';
 let { kind, id, name, resourceVersion, workspaceId, description='', href }: {
  kind:'workspace'|'project';id:string;name:string;resourceVersion:string;workspaceId?:string;description?:string;href:string;
 }=$props();
 let renameDialog: RenameHierarchyDialog;
</script>
<InformationCard title={name} headingLevel={2} navigationHref={href} actions={hierarchyActions(kind, id, () => renameDialog.show(), workspaceId)}><p class="min-h-16 text-sm leading-relaxed text-muted-foreground">{description|| (kind==='workspace'?'Open workspace projects.':'No project goal yet.')}</p></InformationCard>
<RenameHierarchyDialog bind:this={renameDialog} {kind} {id} {name} {resourceVersion} {workspaceId} />
