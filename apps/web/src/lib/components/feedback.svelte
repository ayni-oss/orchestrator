<script lang="ts">
  import { Alert, AlertDescription, AlertTitle } from "$lib/components/ui/alert/index.js";
  let { kind = "info", title, message, requestId }: { kind?: "info" | "success" | "error" | "warning"; title: string; message: string; requestId?: string | undefined } = $props();
</script>

<Alert variant={kind === "error" ? "destructive" : "default"} class={kind === "success" ? "border-emerald-500/40 bg-emerald-500/5" : kind === "warning" ? "border-amber-500/40 bg-amber-500/5" : undefined} role={kind === "error" ? "alert" : "status"}>
  <AlertTitle>{title}</AlertTitle>
  <AlertDescription class="!text-foreground">{message}{#if requestId}<span class="mt-1 block font-mono text-xs">Request {requestId}</span>{/if}</AlertDescription>
</Alert>
