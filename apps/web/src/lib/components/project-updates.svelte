<script lang="ts">
import {onMount,untrack} from "svelte";
import type {ProjectViewData} from "$lib/project-view.js";
import {subscribeRefresh,resourceChanged} from "$lib/refresh.js";
import {watchConversation} from "$lib/conversation-watch.js";
let {data}:{data:ProjectViewData}=$props();
const conversationId=$derived(data.conversation?.id??"");
const projectId=$derived(data.conversation?.projectId??"");
onMount(()=>{const conversation=subscribeRefresh("app:conversations",{intervalMs:120_000}),tasks=subscribeRefresh("app:tasks",{intervalMs:120_000});return ()=>{conversation();tasks();};});
$effect(()=>{
  if(!conversationId)return;
  return watchConversation({projectId,conversationId,cursor:untrack(()=>data.conversation?.updateCursor??"0")},()=>{resourceChanged("app:conversations");resourceChanged("app:tasks");});
});
</script>
