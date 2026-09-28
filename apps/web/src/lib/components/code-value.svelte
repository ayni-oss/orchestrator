<script lang="ts">
  let { value, label }: { value: string; label?: string } = $props();
</script>

<code class="break-all rounded bg-muted px-1.5 py-0.5 text-xs" title={label ?? value}>{value}</code>
