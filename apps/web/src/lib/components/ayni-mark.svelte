<script lang="ts">
 let { class: className = '', orchestrator = false }: { class?: string; orchestrator?: boolean } = $props();
</script>

<span class={['ayni-mark', className, orchestrator && 'orchestrator']} aria-hidden="true"></span>

<style>
 .ayni-mark {
  display: inline-block;
  flex: none;
  background-color: var(--primary);
  mask: url("/ayni.svg") center / contain no-repeat;
 }
 .orchestrator { mask-image: url("/ayni-orchestrator.svg"); }
</style>
