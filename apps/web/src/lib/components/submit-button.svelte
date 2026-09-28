<script lang="ts">
  import { Button, type ButtonProps } from "$lib/components/ui/button/index.js";
  let { pending = false, pendingLabel = "Working…", children, ...props }: ButtonProps & { pending?: boolean; pendingLabel?: string } = $props();
</script>

<Button {...props} type="submit" disabled={pending || props.disabled} aria-busy={pending}>
  {pending ? pendingLabel : ""}
  {#if !pending}{@render children?.()}{/if}
</Button>
