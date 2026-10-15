<script lang="ts">
import type {DevelopmentTaskDto} from "$lib/project-view.js";
import {projectHref} from "$lib/project-navigation.js";
let {task}:{task:DevelopmentTaskDto}=$props();
</script>
<a class="font-medium hover:underline" href={`/tasks/${encodeURIComponent(task.id)}`}>{task.snapshot.request}</a>
<span class="block text-xs text-muted-foreground">{task.snapshot.agent.model}</span>
{#if task.snapshot.origin}<a class="block text-xs underline" href={projectHref(task.snapshot.origin.projectId,"plans",task.snapshot.origin.conversationId,undefined,task.snapshot.origin.planVersion)}>Approved plan · version {task.snapshot.origin.planVersion}</a>{:else}<span class="block text-xs text-muted-foreground">Direct submission · no originating plan</span>{/if}

<a class="block text-xs underline" href={projectHref(task.projectId,"tasks")}>Open project</a>
