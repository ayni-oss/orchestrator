<script lang="ts">
 let { saving = false, dirty = false, pending = false, saved = false }: {
  saving?: boolean; dirty?: boolean; pending?: boolean; saved?: boolean;
 } = $props();
</script>
<p role="status" aria-live="polite" class="text-xs text-muted-foreground">
 {saving ? 'Saving…' : pending ? 'Save not confirmed. Recover the update before making more changes.' : dirty ? 'Unsaved changes' : saved ? 'All changes saved' : 'Changes save automatically'}
</p>
