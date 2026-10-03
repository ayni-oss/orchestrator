<script lang="ts">
  import { onMount } from "svelte";
  import { invalidateAll } from "$app/navigation";
  let busy=false;
  let message=$state("Live updates every 10 seconds");
  onMount(()=>{
    const timer=setInterval(async()=>{
      if(busy || document.visibilityState!=="visible") return;
      busy=true;
      try { await invalidateAll(); message="Live updates every 10 seconds"; }
      catch { message="Updates interrupted — retrying automatically"; }
      finally { busy=false; }
    },10000);
    return ()=>clearInterval(timer);
  });
</script>
<p class="text-xs text-muted-foreground" role="status">{message}</p>
