<script lang="ts">
  import {onMount,untrack} from "svelte";
  import {invalidate} from "$app/navigation";
  import type {PageData} from "../../routes/projects/[projectId]/$types";
  let {data}:{data:PageData}=$props();
  const conversation=$derived(data.conversation);
  let refreshing=false;
  onMount(()=>{
    const timer=setInterval(async()=>{
      if(refreshing || document.visibilityState!=="visible")return;
      refreshing=true;
      try{await Promise.all([invalidate("app:conversations"),invalidate("app:tasks")]);}finally{refreshing=false;}
    },15000);
    return ()=>clearInterval(timer);
  });
  let live=$state(false);
  const selectedConversationId=$derived(conversation?.id??"");
  const selectedProjectId=$derived(conversation?.projectId??"");
  $effect(()=>{
    if(!selectedConversationId)return;
    const initialCursor=untrack(()=>conversation?.updateCursor??"0");
    const source=new EventSource(`/projects/${encodeURIComponent(selectedProjectId)}/conversations/${encodeURIComponent(selectedConversationId)}/updates?after=${encodeURIComponent(initialCursor)}`);
    source.onopen=()=>{live=true;};
    source.onerror=()=>{live=false;};
    source.addEventListener("changed",()=>{void Promise.all([invalidate("app:conversations"),invalidate("app:tasks")]);});
    return ()=>{source.close();live=false;};
  });
</script>
<p role="status" class="text-xs text-muted-foreground">{!selectedConversationId?"Periodic refresh":live?"Live updates":"Reconnecting · periodic refresh"}</p>
